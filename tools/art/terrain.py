"""The ground, the deep rock, the sand and the spikes — the tiles a level is built out of.

Five things in here are worth knowing before editing it.

**The mask is the contract.** `ground_N` is a solid tile whose N is a 4-bit "open to
the air on this side" mask: bit 0 (1) open above, bit 1 (2) open left, bit 2 (4) open
right, bit 3 (8) open below. `crate::view::ground_mask` computes exactly that mask
from a level's neighbours and asks the atlas for `ground_{mask}` (or `deep_{mask}`)
by name, so the *art* is chosen by the mask and nothing else. Sixteen hand-drawn edge
sprites was already more than the old art could keep consistent; the mask is what
makes a level's silhouette come out right without any per-tile art decision.

**A material is a crown, a body and a rock.** Soil is a grass crown over dirt over
stone; sand is a bright loose crown over packed sand over sandstone; and the two are
the *same construction*, which is what this module is built around. It used to be
three hand-written loops (`ground`, `deep`) with the tones spelled out inside them, and
adding a material that way meant a fourth copy of the autotiling geometry — the one
thing that must never be duplicated, because a copy that puts its crown a row lower
inserts a step into a flat floor. So the geometry lives in [`solid`], the tones live in
a [`Material`], and the four families fall out of a table:

| family | surface | body | rock |
|---|---|---|---|
| soil | grass | dirt | stone |
| sand | sand | packed sand | sandstone |

**A solid tile is never transparent.** Sky showing through a solid tile would read as
a hole in the world, so an "open" side is drawn as *shading* — a dark 1px edge and a
recessed crown — never as alpha.

**The crown lives at a fixed height.** Rows 0..[`CROWN_ROWS`] are the surface crown on
every mask that has sky above it, so a run of surface tiles reads as one continuous
surface instead of a row of separate blocks, and the ragged bottom edge of the crown
is what keeps it from reading as a stripe. Every material shares that height, because
it is a *level* property and not a drawing one: one tile of the surface is one tile,
whether it is grass or sand.

**Rock is the same construction in stone**, and exists only for masks with bit 0
clear: rock is never the surface, so the deep families have no crown to draw. What
they do have is their own texture — vertical cracks in stone, horizontal bedding in
sandstone — which is why a material carries a [`Detail`] as well as its tones.

All texture comes from [`h`], a hash-based value noise, so regenerating the atlas can
never produce a diff.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from functools import lru_cache

from PIL import Image

from .common import (
    BRICK,
    LAVA,
    LAVA_CRUST,
    LAVA_DEEP,
    LAVA_GLOW,
    LAVA_HOT,
    LAVA_MID,
    TRANSPARENT,
    VINE,
    VINE_DARK,
    VINE_LIGHT,
    VINE_STEM,
    WATER,
    WATER_DEEP,
    WATER_FOAM,
    WATER_LIGHT,
    WATER_MID,
    BRICK_DARK,
    BRICK_DEEP,
    BRICK_LIGHT,
    BRICK_MID,
    BRICK_TOP,
    DIRT,
    DIRT_DARK,
    DIRT_DEEP,
    DIRT_LIGHT,
    DIRT_MID,
    GRASS,
    GRASS_DARK,
    GRASS_EDGE,
    GRASS_LIGHT,
    GRASS_MID,
    IRON_BOLT,
    IRON_FRAME,
    IRON_LIGHT,
    LANTERN_GLOW,
    LANTERN_WARM,
    MASONRY,
    MASONRY_DARK,
    MASONRY_DEEP,
    MASONRY_LIGHT,
    MASONRY_MID,
    MORTAR,
    MORTAR_DARK,
    SAND,
    SAND_DARK,
    SAND_DEEP,
    SAND_LIGHT,
    SAND_MID,
    SANDSTONE,
    SANDSTONE_DARK,
    SANDSTONE_DEEP,
    SANDSTONE_LIGHT,
    SANDSTONE_MID,
    SHELL,
    SHELL_DARK,
    STONE,
    STONE_DARK,
    STONE_DEEP,
    STONE_LIGHT,
    STONE_MID,
    WOOD,
    WOOD_DARK,
    WOOD_DEEP,
    WOOD_LIGHT,
    WOOD_MID,
    WOOD_TOP,
    cell,
    h,
)

#: An RGBA colour, as `common.py` hands them over.
RGB = tuple[int, int, int, int]

#: How much of a surface tile is crown. Five rows of sixteen is the smallest crown
#: that still reads as grass at the smallest zoom the game allows, and it leaves
#: eleven rows of body for the material and its speckle.
CROWN_ROWS = 5

#: Masks that can appear deep underground: bit 0 (open above) is what makes a tile a
#: surface, so a rock tile is only ever one of these.
DEEP_MASKS = [mask for mask in range(16) if not mask & 0b0001]


class Detail(Enum):
    """What the *texture* of a buried material is made of.

    Not decoration: two materials that share a speckle are two colours of one material.
    Stone is cracked vertically, sandstone is laid down in beds, and that difference is
    the only thing telling an eye which one it is standing on when both are the same
    silhouette at the same zoom.
    """

    NONE = "none"
    CRACKS = "cracks"
    STRATA = "strata"
    BRICKS = "bricks"
    PLANKS = "planks"


@dataclass(frozen=True)
class Crown:
    """A surface's top five rows: the tones, and the two noises that break them up."""

    #: The sky-facing row, the brightest thing in the tile.
    top: RGB
    #: Rows 1 and 2 alternate between these two, in that order.
    light: RGB
    #: Row 3, and the second half of row 2.
    mid: RGB
    #: The noise that breaks rows 1 and 2 up, so the crest is not a flat line.
    seed: int
    #: The noise that decides which columns of the last row drop into the body early.
    ragged_seed: int


@dataclass(frozen=True)
class Body:
    """A material's bulk: four tones and the noise that speckles them.

    `thresholds` is `(light_below, mid_below, dark_above)` against [`h`]'s output. They
    are per material rather than shared because the *texture* is part of what a material
    is: rock is cooler and noisier than dirt, and copying one material's numbers onto
    another is how two materials end up looking like one in a dark underground level.
    """

    light: RGB
    base: RGB
    mid: RGB
    dark: RGB
    deep: RGB
    seed: int
    thresholds: tuple[float, float, float]


@dataclass(frozen=True)
class Material:
    """One ground material: a crown, a body, the colours of its edges, and its texture."""

    #: What the sprites are called: this material is `{name}_{mask}`.
    name: str
    #: The surface, or `None` for a rock: rock is never the surface, so it has no crown.
    crown: Crown | None
    body: Body
    #: The 1px vertical edge, where a tile's side meets open air.
    crown_edge: RGB
    body_edge: RGB
    #: The corner pixel where a crown's top row meets an open side: the darkest tonal step
    #: in the tile, and what rounds the silhouette off at the shoulder.
    corner: RGB
    #: The bottom row of an overhang, and the two tones the row above it is mixed from.
    #: An overhang needs weight under it, or a floating platform reads as a floating
    #: stripe of grass.
    under_deep: RGB
    under_a: RGB
    under_b: RGB
    detail: Detail


# --- the four materials ----------------------------------------------------

#: Grass over dirt, the material every level has had since the first one.
GRASS_CROWN = Crown(GRASS_LIGHT, GRASS, GRASS_MID, seed=7, ragged_seed=41)
DIRT_BODY = Body(
    DIRT_LIGHT,
    DIRT,
    DIRT_MID,
    DIRT_DARK,
    DIRT_DEEP,
    seed=13,
    thresholds=(0.16, 0.30, 0.92),
)
STONE_BODY = Body(
    STONE_LIGHT,
    STONE,
    STONE_MID,
    STONE_DARK,
    STONE_DEEP,
    seed=31,
    thresholds=(0.14, 0.34, 0.90),
)

#: Sand: bright and barely speckled on top, visibly packed a few rows down.
SAND_CROWN = Crown(SAND_LIGHT, SAND, SAND_MID, seed=67, ragged_seed=43)
PACKED_SAND_BODY = Body(
    SAND,
    SAND_MID,
    SAND_DARK,
    SAND_DEEP,
    SAND_DEEP,
    seed=71,
    thresholds=(0.18, 0.36, 0.94),
)
SANDSTONE_BODY = Body(
    SANDSTONE_LIGHT,
    SANDSTONE,
    SANDSTONE_MID,
    SANDSTONE_DARK,
    SANDSTONE_DEEP,
    seed=59,
    thresholds=(0.14, 0.34, 0.90),
)

SOIL = Material(
    name="ground",
    crown=GRASS_CROWN,
    body=DIRT_BODY,
    crown_edge=GRASS_DARK,
    body_edge=DIRT_DARK,
    corner=GRASS_EDGE,
    under_deep=DIRT_DEEP,
    under_a=DIRT_DARK,
    under_b=DIRT_MID,
    detail=Detail.NONE,
)
SAND_GROUND = Material(
    name="sand",
    crown=SAND_CROWN,
    body=PACKED_SAND_BODY,
    crown_edge=SAND_DARK,
    body_edge=SAND_DEEP,
    corner=SAND_DEEP,
    under_deep=SAND_DEEP,
    under_a=SAND_DARK,
    under_b=SAND_MID,
    detail=Detail.NONE,
)
ROCK = Material(
    name="deep",
    crown=None,
    body=STONE_BODY,
    crown_edge=STONE_DARK,
    body_edge=STONE_DARK,
    corner=STONE_DARK,
    under_deep=STONE_DEEP,
    under_a=STONE_DARK,
    # The same tone twice: a rock overhang is flat under there, and saying so with the
    # noise's two colours equal is cheaper than a flag that means "do not mix these".
    under_b=STONE_DARK,
    detail=Detail.CRACKS,
)
SANDSTONE_ROCK = Material(
    name="sandstone",
    crown=None,
    body=SANDSTONE_BODY,
    crown_edge=SANDSTONE_DARK,
    body_edge=SANDSTONE_DARK,
    corner=SANDSTONE_DARK,
    under_deep=SANDSTONE_DEEP,
    under_a=SANDSTONE_DARK,
    under_b=SANDSTONE_DARK,
    detail=Detail.STRATA,
)

#: Brick: terracotta coping stones over mortared courses, over dressed stone masonry.
BRICK_CROWN = Crown(BRICK_TOP, BRICK_LIGHT, BRICK, seed=101, ragged_seed=107)
BRICK_BODY = Body(
    BRICK_LIGHT,
    BRICK,
    BRICK_MID,
    BRICK_DARK,
    BRICK_DEEP,
    seed=109,
    thresholds=(0.15, 0.32, 0.90),
)
MASONRY_BODY = Body(
    MASONRY_LIGHT,
    MASONRY,
    MASONRY_MID,
    MASONRY_DARK,
    MASONRY_DEEP,
    seed=113,
    thresholds=(0.14, 0.34, 0.90),
)

BRICK_GROUND = Material(
    name="brick",
    crown=BRICK_CROWN,
    body=BRICK_BODY,
    crown_edge=BRICK_DARK,
    body_edge=BRICK_DEEP,
    corner=BRICK_DEEP,
    under_deep=BRICK_DEEP,
    under_a=BRICK_DARK,
    under_b=BRICK_MID,
    detail=Detail.BRICKS,
)
MASONRY_ROCK = Material(
    name="masonry",
    crown=None,
    body=MASONRY_BODY,
    crown_edge=MASONRY_DARK,
    body_edge=MASONRY_DARK,
    corner=MASONRY_DARK,
    under_deep=MASONRY_DEEP,
    under_a=MASONRY_DARK,
    under_b=MASONRY_DARK,
    detail=Detail.BRICKS,
)

#: Wood: milled timber decking over horizontal planks and timber beams.
WOOD_CROWN = Crown(WOOD_TOP, WOOD_LIGHT, WOOD, seed=127, ragged_seed=131)
WOOD_BODY = Body(
    WOOD_LIGHT,
    WOOD,
    WOOD_MID,
    WOOD_DARK,
    WOOD_DEEP,
    seed=137,
    thresholds=(0.16, 0.34, 0.92),
)
TIMBER_BODY = Body(
    WOOD,
    WOOD_MID,
    WOOD_DARK,
    WOOD_DEEP,
    WOOD_DEEP,
    seed=139,
    thresholds=(0.14, 0.32, 0.90),
)

WOOD_GROUND = Material(
    name="wood",
    crown=WOOD_CROWN,
    body=WOOD_BODY,
    crown_edge=WOOD_DARK,
    body_edge=WOOD_DEEP,
    corner=WOOD_DEEP,
    under_deep=WOOD_DEEP,
    under_a=WOOD_DARK,
    under_b=WOOD_MID,
    detail=Detail.PLANKS,
)
TIMBER_ROCK = Material(
    name="timber",
    crown=None,
    body=TIMBER_BODY,
    crown_edge=WOOD_DEEP,
    body_edge=WOOD_DEEP,
    corner=WOOD_DEEP,
    under_deep=WOOD_DEEP,
    under_a=WOOD_DARK,
    under_b=WOOD_DARK,
    detail=Detail.PLANKS,
)


# --- the autotiler ----------------------------------------------------------


def _speckle(body: Body, x: int, y: int) -> RGB:
    """One pixel of a material's bulk, from its own noise and thresholds."""
    n = h(x, y, body.seed)
    light_below, mid_below, dark_above = body.thresholds
    if n < light_below:
        return body.light
    if n < mid_below:
        return body.mid
    if n > dark_above:
        return body.dark
    return body.base


def _crown_pixel(crown: Crown, body: Body, x: int, y: int) -> RGB:
    """A crown pixel. `y` is 0 at the sky-facing top of the tile."""
    if y == 0:
        return crown.top
    if y == 1:
        return crown.top if h(x, y, crown.seed) < 0.55 else crown.light
    if y == 2:
        return crown.light if h(x, y, crown.seed) < 0.65 else crown.mid
    if y == 3:
        return crown.mid
    # The last row of the crown is the ragged boundary: some columns drop into the
    # body early, which is what stops a straight line reading as a painted stripe.
    return body.mid if h(x, y, crown.ragged_seed) < 0.42 else crown.mid


def solid(mask: int, material: Material) -> Image.Image:
    """One solid tile in `material`: a crown if the sky is above it, its body under that.

    The whole of the autotiling geometry, and the only copy of it: which rows are crown,
    which columns are edge, and what an overhang does are decisions about the *shape* of
    a level rather than about what it is made of, so every material inherits them.
    """
    img = cell()
    px = img.load()
    crown = material.crown
    open_up = bool(mask & 0b0001) and crown is not None
    for y in range(16):
        for x in range(16):
            if open_up and y < CROWN_ROWS:
                px[x, y] = _crown_pixel(crown, material.body, x, y)
            else:
                # The first row under the crown is the body's mid tone: it is the shadow
                # the surface throws on itself, and it is what makes a buried tile read
                # as *under* something rather than as a second surface.
                px[x, y] = material.body.mid if y == 0 else _speckle(material.body, x, y)

    # A side that meets open air is an edge, and the crown recedes at it: the top
    # corner is the darkest tone, which is what rounds the silhouette off.
    for side in (0b0010, 0b0100):
        if mask & side:
            column = 0 if side == 0b0010 else 15
            for y in range(16):
                px[column, y] = (
                    material.crown_edge if (open_up and y < CROWN_ROWS) else material.body_edge
                )
            if open_up:
                px[column, 0] = material.corner

    if mask & 0b1000:
        for x in range(16):
            px[x, 15] = material.under_deep
            px[x, 14] = material.under_a if h(x, 14, 53) < 0.6 else material.under_b

    if material.detail is Detail.BRICKS:
        _bricks(px, material, open_up, mask)
    elif material.detail is Detail.PLANKS:
        _planks(px, material, open_up, mask)

    return img


def _bricks(px, material: Material, open_up: bool, mask: int) -> None:
    """Coursed brickwork or stone ashlar: horizontal mortar joints and staggered vertical joints."""
    body = material.body
    start_y = CROWN_ROWS if open_up else 1
    end_y = 14 if (mask & 0b1000) else 15
    x0 = 1 if (mask & 0b0010) else 0
    x1 = 15 if (mask & 0b0100) else 16
    for y in range(start_y, end_y):
        if y in (5, 9, 13):
            for x in range(x0, x1):
                if h(x, y, 163) < 0.85:
                    px[x, y] = body.dark
        else:
            course = (y - 1) // 4
            offset = 4 if (course % 2 == 1) else 0
            for x in range(x0, x1):
                if (x + offset) % 8 == 0 and x0 < x < x1 - 1:
                    px[x, y] = body.deep
                elif y in (6, 10) and (x + offset) % 8 in (1, 2):
                    px[x, y] = body.light


def _planks(px, material: Material, open_up: bool, mask: int) -> None:
    """Horizontal wood grain, plank seams, and recessed iron bolts."""
    body = material.body
    start_y = CROWN_ROWS if open_up else 1
    end_y = 14 if (mask & 0b1000) else 15
    x0 = 1 if (mask & 0b0010) else 0
    x1 = 15 if (mask & 0b0100) else 16
    for y in range(start_y, end_y):
        if y in (7, 11):
            for x in range(x0, x1):
                px[x, y] = body.deep
        elif y in (8, 12):
            for x in range(x0, x1):
                if h(x, y, 173) < 0.7:
                    px[x, y] = body.light
    for bx, by in ((3, 6), (12, 6), (3, 10), (12, 10)):
        if start_y <= by < end_y and x0 <= bx < x1:
            px[bx, by] = body.deep


def _cracks(px, material: Material) -> None:
    """Two or three vertical cracks, so a wall of rock is not a flat field of speckle."""
    body = material.body
    for x in range(16):
        if h(x, 0, 61) < 0.18:
            long = 3 + int(h(x, 1, 67) * 4)
            for step in range(long):
                y = 4 + step
                if y < 15:
                    px[x, y] = body.deep
    return


def _strata(px, material: Material) -> None:
    """Horizontal bedding planes: sandstone is stone that was laid down in layers."""
    body = material.body
    for y in (4, 8, 12):
        for x in range(16):
            if h(x, y, 83) < 0.72:
                px[x, y] = body.deep
            elif h(x, y + 1, 89) < 0.4:
                px[x, y] = body.dark
    # And a few grains standing proud of a bed, so the layers are not three straight lines.
    for x, y in ((3, 3), (9, 7), (12, 11), (6, 13)):
        px[x, y] = body.light


def rock(mask: int, material: Material) -> Image.Image:
    """One buried tile. `mask` never has bit 0 set: rock is never the surface."""
    assert not mask & 0b0001, "rock is never the surface"
    assert material.crown is None, "a rock material has no crown to draw"
    img = solid(mask, material)
    if material.detail is Detail.CRACKS:
        _cracks(img.load(), material)
    elif material.detail is Detail.STRATA:
        _strata(img.load(), material)
    return img


def ground(mask: int) -> Image.Image:
    """One soil tile: a grass crown if the sky is above it, dirt under the crown."""
    return solid(mask, SOIL)


def deep(mask: int) -> Image.Image:
    """One stone tile."""
    return rock(mask, ROCK)


def sand(mask: int) -> Image.Image:
    """One sand tile: a bright crown of loose sand over packed sand."""
    return solid(mask, SAND_GROUND)


def sandstone(mask: int) -> Image.Image:
    """One sandstone tile: the rock under a beach."""
    return rock(mask, SANDSTONE_ROCK)


def brick(mask: int) -> Image.Image:
    """One brick tile: terracotta coping over mortared brick courses."""
    return solid(mask, BRICK_GROUND)


def masonry(mask: int) -> Image.Image:
    """One masonry tile: dressed stone under a brick structure."""
    return rock(mask, MASONRY_ROCK)


def wood(mask: int) -> Image.Image:
    """One wood plank tile: milled timber decking over planked beams."""
    return solid(mask, WOOD_GROUND)


def timber(mask: int) -> Image.Image:
    """One deep timber tile: heavy structural beams under wood decking."""
    return rock(mask, TIMBER_ROCK)


def _with_detail(base: Image.Image, paint) -> Image.Image:
    """Copy a base tile and add pixels to it.

    Variants are built this way *on purpose*: a variant that redrew the crown could
    put its grass a row off and break the surface line of a whole level, so the
    detail is only ever added on top of the tile it varies.
    """
    img = base.copy()
    paint(img.load())
    return img


def _flowers(px) -> None:
    """Three small flowers on the crown, at fixed columns so they are reproducible."""
    for x, petal in ((3, DIRT_LIGHT), (8, DIRT_LIGHT), (12, DIRT_LIGHT)):
        px[x, 1] = petal
        px[x, 2] = GRASS_DARK
        px[x + 1, 1] = petal
        px[x, 2] = GRASS_DARK
    px[7, 2] = GRASS_DARK
    px[13, 3] = GRASS_DARK


def _tuft_and_pebble(px) -> None:
    """A tuft of longer grass and a pebble sitting on the crown."""
    for x in (5, 6, 6):
        px[x, 1] = GRASS_DARK
    px[6, 0] = GRASS
    px[11, 2] = DIRT_LIGHT
    px[12, 2] = DIRT_LIGHT
    px[11, 3] = DIRT_MID


def _pebbles(px, material: Material = ROCK) -> None:
    """Lighter stones in a rock tile, the way a dirt variant gets its own texture."""
    for x, y in ((4, 3), (5, 3), (4, 4), (10, 7), (11, 7), (9, 12), (10, 12), (10, 13)):
        px[x, y] = material.body.light
    for x, y in ((4, 5), (11, 8), (9, 13)):
        px[x, y] = material.body.dark


def _crack(px, material: Material = ROCK) -> None:
    """A diagonal crack, so one rock tile in three is visibly not the others."""
    for step in range(9):
        x = 6 + step
        y = 2 + step
        px[x, y] = material.body.deep
        if step % 3 == 0:
            px[x, y + 1] = material.body.dark
    px[13, 4] = material.body.light
    px[12, 13] = material.body.dark


def _ripples(px) -> None:
    """Wind ripples across the beach, with the crests catching the light.

    The sand's variant, and the same bargain the grass variants make: the first four rows
    are its own and the fifth is not. Row 4 is where the crown is ragged against the body
    — the *silhouette* of the surface rather than its colour — and a variant that moved a
    pixel there would put a step in a flat beach.
    """
    for y in (1, 2):
        for x in range(1, 15):
            if h(x, y, 97) < 0.75:
                px[x, y] = SAND_LIGHT
            if h(x, y + 1, 103) < 0.4:
                px[x, y + 1] = SAND_MID
    for x in (2, 7, 13):
        px[x, 3] = SAND_DARK


def _shells(px) -> None:
    """A shell and a scatter of grit: what a beach collects that a lawn does not."""
    # A little spiral, drawn rather than stamped: two whorls and a lip. Rows 0..3 only,
    # the same as the ripples: row 4 is the beach's silhouette.
    for x, y in ((4, 1), (5, 1), (3, 2), (4, 2), (5, 2), (6, 2), (3, 3), (4, 3), (5, 3)):
        px[x, y] = SHELL
    px[5, 2] = SHELL_DARK
    px[4, 3] = SHELL_DARK
    px[6, 3] = SHELL_DARK
    # Grit: darker grains where the wind has sorted them.
    for x, y in ((10, 0), (12, 1), (9, 2), (13, 2), (11, 3)):
        px[x, y] = SAND_DARK


def _brick_moss(px) -> None:
    """Lichen and weathered chips on the brick coping."""
    for x in (3, 4, 11, 12):
        px[x, 1] = GRASS_MID
    px[4, 2] = GRASS_DARK
    px[11, 2] = GRASS_DARK
    px[7, 2] = BRICK_DARK
    px[8, 3] = BRICK_DARK


def _brick_cracked(px) -> None:
    """A weathered notch and mortar fleck on the brick coping."""
    px[5, 1] = BRICK_DARK
    px[5, 2] = BRICK_DEEP
    px[6, 2] = BRICK_DARK
    px[10, 1] = MORTAR
    px[11, 1] = MORTAR_DARK
    px[10, 3] = BRICK_DARK


def _wood_knots(px) -> None:
    """Wood grain knots and iron nail heads on the timber deck."""
    px[4, 1] = WOOD_DARK
    px[5, 1] = WOOD_DEEP
    px[5, 2] = WOOD_DARK
    px[11, 2] = IRON_BOLT
    px[12, 2] = IRON_LIGHT


def _wood_rivets(px) -> None:
    """Countersunk iron rivets along the top decking plank."""
    for x in (2, 7, 13):
        px[x, 1] = IRON_LIGHT
        px[x, 2] = IRON_BOLT
    px[9, 3] = WOOD_DARK


# --- blending: the seam where two materials meet ----------------------------
#
# This is the module's second contract with the renderer, and it is a contract about
# *materials* rather than about tiles: where two of them meet, each creeps a few pixels
# into the other's tile, and the seam stops being a line.
#
# Two decisions make it cheap, and both of them are worth stating before the code.
#
# **The band is a mask over the material's own cell.** A spill is not drawn: it is the
# material's plain interior cell with everything but a ragged edge band made transparent.
# So a beach's spill *is* beach pixels — there is no second palette in which to pick a
# slightly-wrong yellow — and the way a material looks at a seam cannot drift away from
# the way it looks everywhere else.
#
# **A material, not a pair of materials.** Thirty-eight bands cover every seam the ten
# looks can make, in both directions, because the band belongs to the material that
# is crossing rather than to the pair it is crossing into. A pair table would need four
# more bands for every material already shipped, to say the same thing twice; a sixth
# material costs its own four bands and no new code and no change to the renderer. That
# is exactly how brick and timber arrived: a masonry texture, a plank texture, and the
# eight bands below, with nothing in the Rust view changed but a lookup row.

#: The ten things a solid tile can look like, and the material each is made of.
#:
#: A material has three appearances — a crown where the sky touches it, a body a few
#: tiles down, and rock under that — and two of the ten are one material's crown and
#: body. Those two *already* join seamlessly, because the crown's last row is drawn
#: ragged against the body on purpose ([`CROWN_ROWS`]), so a blend is only ever drawn
#: where two different materials meet: grass over dirt is a surface, sand beside dirt is
#: a seam. Bricks and planks arrived as their own materials, so they carry their own
#: seams: a brick wall meeting a bank of soil used to be a hard line.
LOOKS = ("grass", "dirt", "stone", "beach", "packed", "sandstone", "brick", "masonry", "wood", "timber")
MATERIAL_OF = {
    "grass": "soil",
    "dirt": "soil",
    "stone": "stone",
    "beach": "sand",
    "packed": "sand",
    "sandstone": "sandstone",
    "brick": "masonry",
    "masonry": "masonry",
    "wood": "timber",
    "timber": "timber",
}

#: How far each material creeps into the tile beside it, in pixels.
#:
#: Looseness rather than decoration: sand is a pile of grains and spills five pixels,
#: soil crumbles three, sandstone two, and stone — which is stone — barely moves at all.
#: Five is a third of a tile, which is as far as a band can reach before two of them
#: meeting in the middle turns a seam into a smear. Masonry is dressed stone and holds its
#: edge like stone does; timber is milled and splinters at its cut ends, so it travels with
#: more than stone and less than raw earth.
SPILL = {"soil": 3, "sand": 5, "stone": 2, "sandstone": 3, "masonry": 2, "timber": 3}

#: The four sides a band can lie along, and the two looks that are never asked for on
#: one of them. A surface tile has sky above it, so no solid tile ever has a crown
#: *below* it: `blend_grass_b` and `blend_beach_b` are the two bands that describe a
#: situation the level format cannot build, and they are the two the atlas does not ship.
#:
#: Grass and beach only, even though brick and wood are crowns too: a look is a *depth*,
#: not a tile, and `brick` and `wood` are the shallow-body looks as well — the tile two
#: courses under a brick coping is drawn as `brick`, so it can perfectly well have a
#: different material below it and needs its `_b` band to say so. Grass and dirt are two
#: looks for exactly this reason, and the sand family learned to be two at the same time.
SIDES = ("l", "r", "t", "b")
BLIND_SIDES = {"grass": ("b",), "beach": ("b",)}


def _plain(look: str) -> Image.Image:
    """The cell a spill is cut from: the material's own plain interior tile.

    `mask` is the one a tile has when nothing is open around it, and for the two surface
    looks the bit that means "sky above" — a band along a vertical edge carries the crown
    at its top rows and the body below, which is exactly the shape of the tile it came
    from and exactly what makes a grass seam green at the surface line and brown under it.
    """
    plain = {
        "grass": lambda: solid(0b0001, SOIL),
        "dirt": lambda: solid(0, SOIL),
        "stone": lambda: rock(0, ROCK),
        "beach": lambda: solid(0b0001, SAND_GROUND),
        "packed": lambda: solid(0, SAND_GROUND),
        "sandstone": lambda: rock(0, SANDSTONE_ROCK),
        "brick": lambda: solid(0b0001, BRICK_GROUND),
        "masonry": lambda: rock(0, MASONRY_ROCK),
        "wood": lambda: solid(0b0001, WOOD_GROUND),
        "timber": lambda: rock(0, TIMBER_ROCK),
    }
    if look not in plain:
        raise ValueError(f"unknown look {look!r}")
    return plain[look]()


def _at(side: str, along: int, depth: int) -> tuple[int, int]:
    """The pixel `depth` pixels in from `side`, `along` the edge from its start."""
    if side == "l":
        return depth, along
    if side == "r":
        return 15 - depth, along
    if side == "t":
        return along, depth
    return along, 15 - depth


def blend(look: str, side: str) -> Image.Image:
    """One material's granules crossing one edge into the tile next door.

    Transparent except a scatter of this material's own pixels up to [`SPILL`] deep along
    `side` — densest at the edge, thinning inward, and **not necessarily there at all**:
    a row reaches zero pixels, one, or all the way, and the grains inside a row it does
    reach are dropped at a rate that rises with depth.

    That dither is the whole point, and getting it wrong is what a *seam* is. Each band
    is drawn over the tile's own opaque cell, so what a seam looks like is decided by the
    two bands meeting across the tile line: if both are solid at their own edge — which is
    how this was first written, on the theory that the two materials have to touch — the
    two of them stack into `A B | A B`: a stripe of the neighbour inside each tile, two
    hard parallel lines a pixel apart, and the eye reads *both* of them. Sand against
    soil, whose tones are five steps apart, is then a bright stripe with a dark one beside
    it for the whole height of the ground, which is the seam this was supposed to remove.
    Dithering both sides instead lets the materials *interlock*: the boundary wanders a
    few pixels either way with grains of each material on the other side of it, which is
    what a pile of sand meeting a bank of soil looks like.

    Every opaque pixel is still the material's own, read from its plain cell at the same
    coordinate, so the band is a continuation of the material rather than a picture of
    it: at a seam the grass line simply carries on a few ragged pixels into the sand.
    """
    source = _plain(look)
    depth = SPILL[MATERIAL_OF[look]]
    seed = 151 + LOOKS.index(look) * 11 + SIDES.index(side) * 5
    img = cell()
    dst = img.load()
    for along in range(16):
        # How far this row of the seam reaches: `0` leaves the tile's own material at
        # its own edge, so the two bands do not both cover the tile line and stack into
        # a stripe each. A third of the rows on average keep their own edge.
        reach = int(h(along, 0, seed) * (depth + 1))
        for d in range(reach):
            # Grains: a row that crosses the edge at all crosses it on its first pixel,
            # and reaches further the rarer, so a band has a dense edge and a speckled
            # tail rather than the straight inner boundary a mask cut would leave.
            keep = 1.0 - 0.5 * (d / depth)
            if h(along, d + 1, seed) > keep:
                continue
            x, y = _at(side, along, d)
            dst[x, y] = source.getpixel((x, y))
    return img


# --- the two trap tiles -----------------------------------------------------
#
# Both of these are drawn *over* something rather than instead of it, and that is the whole of
# how they relate to the materials they belong to.
#
# A cracked tile is its material with a **transparent** crack drawn on top, so the renderer can
# put the crack over the material's own autotile: a brittle beach is still drawn as a beach —
# its crown, its edges, its seams with the lawn beside it — and the crack says the one thing the
# material's picture cannot. Cutting whole `cracked_*` cells instead would mean every brittle
# tile looked like a lone block, because a 16-mask family is what tells a floor from a pillar.
#
# The spring is the opposite: it is an *object* standing in a cell, like the spike, so it is
# drawn whole with air around it and shares the spike's one drawing rule — nothing touches the
# top of the cell, because a tile that reaches its ceiling stops reading as a thing standing in
# the cell and starts reading as part of the wall above it. The sim's launch zone follows that
# gap (`physics::SPRING_TOP_CLEARANCE`), so what a body lands on is what it can see.

#: How many rows at the top of a pad's cell are air. Must match `physics::SPRING_TOP_CLEARANCE`
#: in the sim, which is the copy the *game* reads: `4 / 16` there and four rows here are the same
#: statement, and `REVIEW.md` §2.12 is where the cost of letting the two drift was measured.
SPRING_AIR_ROWS = 4

#: The deep shade at the heart of a crack, and the dark one along its shoulders, per material.
#: A crack is a shadow a material throws on itself, so it is drawn in that material's own tones:
#: a grey fissure across a beach is the kind of detail that stops the eye reading sand.
CRACK_TONES = {
    "ground": (DIRT_DEEP, DIRT_DARK),
    "sand": (SAND_DEEP, SAND_DARK),
    "brick": (BRICK_DEEP, BRICK_DARK),
    "wood": (WOOD_DEEP, WOOD_DARK),
}


def crack_overlay(look: str) -> Image.Image:
    """One material's crack, on a transparent cell, for the renderer to lay over its autotile.

    A fissure rather than a scratch, and **one pixel wide**: it walks down the cell drifting one
    pixel sideways every couple of rows, its heart is the material's deepest shade, and a shoulder
    pixel of the dark tone appears beside it now and then so the line has grain. The width is the
    whole measurement — on brick or decking, whose own texture is a busy grid of mortar and plank
    seams, a two-pixel crack stops being a crack and becomes a smudge that hides the material.

    One branch leaves the fissure partway down and drifts away from it, which is what makes the
    mark read as something that *happened* to the ground rather than something drawn on it.
    """
    heart, shoulder = CRACK_TONES[look]
    img = cell()
    px = img.load()
    seed = 197 + sorted(CRACK_TONES).index(look) * 13
    for start in (3, 10):
        if h(start, 0, seed) < 0.25:
            continue
        x = start
        drift = 1 if h(start, 9, seed + 23) < 0.5 else -1
        for y in range(3, 15):
            px[x, y] = heart
            if h(x, y, seed + 7) < 0.18:
                px[x + drift, y] = shoulder
            if h(x, y, seed + 5) < 0.45:
                x += drift
            if h(x, y, seed + 29) < 0.12:
                drift = -drift
            x = max(1, min(14, x))
        bx, by = start, 7 + int(h(start, 2, seed) * 4)
        for step in range(2 + int(h(start, 3, seed) * 3)):
            bx -= drift
            by += 1
            if 0 <= bx < 16 and by < 15:
                px[bx, by] = heart if step == 0 else shoulder
    return img


def spring() -> Image.Image:
    """A spring pad: a coiled column between two steel plates, standing in its cell.

    **Nothing above** [`SPRING_AIR_ROWS`] **is drawn**, and the sim's launch zone starts where the
    drawing does. That is the same contract the spike's top three rows keep with
    `physics::spike_lethal_box`: the art is what the player can see, and the tile's behaviour is
    aligned to it rather than the other way round.

    Warm brass for the coil and cold steel for the plates — the same warm ink the lantern and the
    flag use, which is what makes a pad read as a *friendly* object in a level full of grey spikes.
    """
    img = cell()
    px = img.load()
    top = SPRING_AIR_ROWS
    # The cap: the plate a body lands on, lit along its top edge.
    for x in range(1, 15):
        px[x, top] = IRON_LIGHT
        px[x, top + 1] = IRON_BOLT
    # The coil: three bars, each with a short connector at one end, the end alternating so the
    # zig-zag between the bars is what reads as a wound spring rather than a stack of stripes.
    bars = list(range(top + 3, 13, 2))
    for index, y in enumerate(bars):
        left = 3 if index % 2 == 0 else 4
        right = 12 if index % 2 == 0 else 13
        for x in range(left, right + 1):
            px[x, y] = LANTERN_WARM
        px[left, y] = LANTERN_GLOW
        px[right, y + 1] = IRON_FRAME
        connector = left - 1 if index % 2 == 0 else right + 1
        for gap in range(y + 1, min(y + 3, 14)):
            px[connector, gap] = IRON_BOLT
    # The base plate, which is what a pad is bolted to the ground with.
    for x in range(0, 16):
        px[x, 15] = IRON_FRAME
        px[x, 14] = IRON_BOLT
    for x in range(1, 15):
        px[x, 13] = IRON_LIGHT if h(x, 13, 181) < 0.85 else IRON_BOLT
    return img


def spikes() -> Image.Image:
    """Three upward spikes on a stone base.

    The top row stays transparent so a spike tile cannot appear to touch the tile
    above it: a hazard that looks like part of the ceiling stops reading as a hazard.
    """
    img = cell()
    px = img.load()
    base = 12
    for y in range(base, 16):
        for x in range(16):
            n = h(x, y, 29)
            px[x, y] = STONE_DEEP if n < 0.30 else STONE_DARK
    for x in range(16):
        px[x, base] = STONE_MID

    for index in range(3):
        centre = 2.5 + index * 5.5
        apex = 3
        for y in range(apex, base):
            t = (y - apex) / (base - apex)
            half = 0.4 + t * 2.0
            for x in range(16):
                offset = (x + 0.5) - centre
                if abs(offset) <= half:
                    if offset < -half * 0.25:
                        px[x, y] = STONE_LIGHT
                    elif abs(offset) < half * 0.4:
                        px[x, y] = STONE
                    else:
                        px[x, y] = STONE_DARK
        px[int(centre), apex] = STONE_LIGHT
        px[int(centre) + 1, apex + 1] = STONE_LIGHT
    return img


def oneway(part: str) -> Image.Image:
    """A one-way platform: a timber shelf across the **top** of its cell.

    The height is the whole point. A one-way platform is ground from above and air from
    below, so the art has to be a thin thing at the top of the tile a body stands on, and
    not a block that fills the cell: a full cell would look like a wall the body walks
    *through*, which is exactly the bug the tile exists to avoid looking like.

    Three pieces (left end, middle, right end) so a run of them reads as one continuous
    deck with a cut end, rather than as a row of separate tiles with a seam between each
    pair. The brackets underneath are what say "shelf" instead of "floor tile that happens
    to be short": they are the only part of the drawing that hangs into the air.
    """
    img = cell()
    px = img.load()
    left = 0 if part != "l" else 1
    right = 15 if part != "r" else 14

    for x in range(left, right + 1):
        px[x, 0] = WOOD_TOP if h(x, 0, 311) < 0.8 else WOOD_LIGHT
        px[x, 1] = WOOD
        px[x, 2] = WOOD_DARK
    # The cut end: a darker column where the deck stops, so the two ends of a run are
    # visible from the side a body arrives from.
    if part == "l":
        for y in range(0, 3):
            px[0, y] = WOOD_DARK
    if part == "r":
        for y in range(0, 3):
            px[15, y] = WOOD_DARK

    # Brackets: iron, two cells in from each end that is not a cut end.
    for x in ((2, 3) if part != "r" else () ):
        for y in range(3, 6):
            px[x, y] = IRON_BOLT
        px[x, 3] = IRON_LIGHT
    for x in ((12, 13) if part != "l" else ()):
        for y in range(3, 6):
            px[x, y] = IRON_BOLT
        px[x, 3] = IRON_LIGHT

    # A warm rivet at each bracket's foot reads as a thing that is *hung* rather than set.
    for x in ((2, 13) if part == "m" else ()):
        px[x, 5] = LANTERN_WARM
    return img


def mover(part: str) -> Image.Image:
    """A moving platform: an iron-railed shelf, drawn in the **top half** of its cell.

    Half a cell is the platform's whole physics box ([`sim::physics::platform_h`]), so the
    art is confined to rows 0..7: the rows below it are the air the shelf slides through,
    and a drawing that filled the cell would look like a wall that moves.

    The rail is what distinguishes it from the static one-way deck at a glance. The two
    are the same shape of thing on purpose — both are ground from above and air from
    below — and what separates them is that one is bolted to nothing and slides.
    """
    img = cell()
    px = img.load()
    left = 1 if part == "l" else 0
    right = 14 if part == "r" else 15

    for x in range(left, right + 1):
        px[x, 0] = IRON_LIGHT
        px[x, 1] = IRON_BOLT
        px[x, 2] = WOOD_TOP
        px[x, 3] = WOOD
        px[x, 4] = WOOD_DARK
    for x in range(left, right + 1, 3):
        px[x, 1] = IRON_FRAME
    if part != "l":
        px[0, 2] = IRON_FRAME
    if part != "r":
        px[15, 2] = IRON_FRAME

    # The chain: two links hanging off the underside, at the middle only, because a lift
    # that is drawn hanging everywhere is a lift that is not moving.
    if part == "m":
        for y in (6, 7):
            px[7, y] = IRON_BOLT
            px[8, y] = IRON_FRAME
    return img


#: The water surface's crest height, column by column, for the two frames.
#: A table rather than a sine so the wave is a drawing rather than a maths problem:
#: the second frame is the first rotated by eight columns, which is what makes a crest
#: appear to travel along a long pool instead of the whole surface bobbing in place.
WATER_CREST = [1, 1, 2, 2, 3, 2, 1, 1, 1, 2, 3, 3, 2, 1, 1, 1]


def _blob(x: int, y: int, period: int, seed: int, wrap: int = 0) -> float:
    """Smooth value noise in [0, 1): a hash at every `period` pixels, curved in between.

    [`h`] on its own is white noise, so hashing one lattice point per three pixels is not a
    blob — it is a three-pixel checkerboard, and a body made of those stacks into a visible
    grid of identical squares. Interpolating between the lattice points with a smoothstep is
    what turns the same hash into patches with a shape and no lattice to see.

    `wrap` is how many lattice steps make a period, so the field joins up with itself after
    `wrap * period` pixels. A fluid's body is a *texture the level tiles*, and a texture that
    does not join up draws a seam every time it repeats — which is the whole reason the
    argument exists.
    """
    fx, fy = x / period, y / period
    x0, y0 = int(fx), int(fy)
    tx, ty = fx - x0, fy - y0
    tx = tx * tx * (3 - 2 * tx)
    ty = ty * ty * (3 - 2 * ty)

    def at(ix: int, iy: int) -> float:
        if wrap:
            ix %= wrap
            iy %= wrap
        return h(ix, iy, seed)

    top = at(x0, y0) * (1 - tx) + at(x0 + 1, y0) * tx
    bottom = at(x0, y0 + 1) * (1 - tx) + at(x0 + 1, y0 + 1) * tx
    return top * (1 - ty) + bottom * ty


#: How many cells across and down a fluid's body texture is drawn as.
#:
#: **One is not enough, and a variant per cell is not the answer.** A fluid's body has to be
#: seamless — two stacked cells are one body of water — so a single seamless cell is the *same
#: picture* in every cell of a pool, and that repeat is what the eye picks out of a wide pool. A
#: per-cell variant (the ground's `_b`/`_c` trick) moves the problem instead of solving it: two
#: different textures meeting at a cell boundary draw a line there. So the fluid is drawn as one
#: picture `FLUID_BLOCK` cells square, cut into cells and indexed by the cell's own position
#: modulo `FLUID_BLOCK` (`crate::view::fluid_body_sprite`): there is no seam at any cut, because
#: the picture is continuous across them, and the repeat is four tiles instead of one.
FLUID_BLOCK = 4


def _fluid_texture(
    base,
    shade,
    light,
    deep,
    seed: int,
    shade_at: float = 0.44,
    light_at: float = 0.64,
    glints: int = 14,
) -> Image.Image:
    """One fluid's body, `FLUID_BLOCK` cells square, drawn as a single continuous picture.

    **No row may depend on its height, and no cell may carry structure of its own**: this
    picture is cut into cells and tiled by the renderer, so a gradient or a bright edge in it
    would be drawn once per cell — a band straight across the middle of a pool, which is the
    bug this family was reshaped to remove. Depth, heat and shape all belong to the cells a
    pool *ends* in ([`water_top`], [`water_bed`], [`lava_top`], [`lava_bed`]), which occur
    exactly once per column.

    Three noise scales: two octaves of [`_blob`] give the fluid currents of its own at a
    distance, the per-pixel grain breaks the patches up so the body is a fluid rather than a
    set of clouds, and the short glints are the only thing in here with a direction — a short
    horizontal glint is a highlight, while a long one would be a line.
    """
    size = FLUID_BLOCK * 16
    img = Image.new("RGBA", (size, size), TRANSPARENT)
    px = img.load()
    for y in range(size):
        for x in range(size):
            patch = 0.65 * _blob(x, y, 8, seed, size // 8) + 0.35 * _blob(
                x, y, 4, seed + 3, size // 4
            )
            grain = h(x, y, seed + 7)
            tone = base
            if patch < shade_at:
                tone = shade
            elif patch > light_at:
                tone = light
            if grain < 0.07:
                tone = deep
            elif grain > 0.985:
                # Rare on purpose: a bright pixel every few of them is confetti, and the
                # patch scale above is where the light belongs.
                tone = light
            px[x, y] = tone
    for index in range(glints):
        gx = int(h(index, 1, seed + 11) * (size - 4)) + 1
        gy = int(h(index, 2, seed + 12) * (size - 1))
        for step in range(2 + int(h(index, 3, seed + 13) * 2)):
            px[min(gx + step, size - 1), gy] = light
    return img


def _fluid_cells(texture: Image.Image) -> list[Image.Image]:
    """A fluid picture cut into cells, row-major: cell `(r, c)` is `r * FLUID_BLOCK + c`."""
    return [
        texture.crop((c * 16, r * 16, c * 16 + 16, r * 16 + 16))
        for r in range(FLUID_BLOCK)
        for c in range(FLUID_BLOCK)
    ]


@lru_cache(maxsize=None)
def water_cells() -> list[Image.Image]:
    """Water's sixteen body cells. **Copy one before drawing on it.**"""
    # The bright end is rare, and the *glints* are where the light is: a body built mostly of
    # highlights reads as foam, and foam is the one thing a pool is not made of.
    return _fluid_cells(
        _fluid_texture(
            WATER, WATER_MID, WATER_LIGHT, WATER_DEEP, 311, shade_at=0.42, light_at=0.86
        )
    )


@lru_cache(maxsize=None)
def lava_cells() -> list[Image.Image]:
    """Lava's sixteen body cells. **Copy one before drawing on it.**"""
    # Molten, and drawn from the molten end of the palette: the *body* of a lava pool is the hot
    # part, and the crust is what cools against the air and the ground — which is what the
    # surface, the bed and the walls are for. A body built from the crust end instead reads as
    # rusty soil, which is what the first version of this drawing did.
    # The shade is the rarest of the three tones on purpose: `LAVA_MID` against `LAVA` is a big
    # step, and a third of a pool in it reads as a leopard skin rather than as molten rock. The
    # dark end is left to the bed, the walls and the single-pixel crust specks.
    return _fluid_cells(
        _fluid_texture(
            LAVA, LAVA_MID, LAVA_HOT, LAVA_DEEP, 331, shade_at=0.26, light_at=0.86
        )
    )


def water() -> Image.Image:
    """The **plain** body of a pool: cell 0 of water's texture.

    Opaque, unlike a spike or a pad, because it is not an object standing in a cell — it is the
    *cell*. A pool's silhouette is the ground it is cut into, which the autotiled terrain draws;
    what the fluid owns is its texture, its surface ([`water_top`]), its bed ([`water_bed`]) and
    its edges ([`_fluid_wall`]).

    This plain cell has two callers: a tileset that has none of the sixteen block cells (they
    are optional art), and the surfaces, which are a body with a crest drawn over it.
    """
    return water_cells()[0].copy()


def water_bed() -> Image.Image:
    """A pool's bed: the body, settling into the floor, drawn **over** the body cell.

    An overlay rather than a whole cell, and that is the mechanic the terrain has had all
    along: the bed is a *drawing on top of* a body tile, so it composes with all three body
    textures instead of existing three times over, and the renderer draws it only where the
    tile below is not water (`crate::view::draw_fluid_edges`).

    The rows above it are transparent, and the shading's top edge is ragged — coverage by
    row is a *density* (18%, 50%, 85%, all of it) rather than a band, so a pool settles into
    its floor instead of ending on a ruled line.
    """
    img = cell()
    px = img.load()
    for x in range(16):
        for y in range(12, 16):
            if h(x, y, 361) > (0.18, 0.5, 0.85, 1.0)[y - 12]:
                continue
            if y == 12:
                px[x, y] = (*WATER_MID, 200)
            elif y == 13:
                px[x, y] = (*WATER_MID, 255)
            elif y == 14:
                px[x, y] = (*(WATER_MID if h(x, y, 362) < 0.7 else WATER), 255)
            else:
                px[x, y] = (*(WATER_DEEP if h(x, y, 362) < 0.7 else WATER_MID), 255)
    return img


def water_top(frame: int) -> Image.Image:
    """The surface of a pool: a crest line with foam, over a body of water.

    The rows above the crest stay **transparent**, because the cell above a surface is air —
    a filled top row would put the waterline at the top of the tile no matter where the wave
    was drawn, which is the same mistake the spike's top row exists to avoid.
    """
    img = water()
    px = img.load()
    shift = 0 if frame == 0 else 8
    for x in range(16):
        crest = WATER_CREST[(x + shift) % 16]
        for y in range(crest):
            px[x, y] = TRANSPARENT
        px[x, crest] = WATER_FOAM
        if crest > 0 and h(x, crest, 319) < 0.22:
            px[x, crest - 1] = WATER_FOAM
        # The lit band under the crest **fades back into the body within four rows**, so the
        # surface cell's own bottom is indistinguishable from the body cell under it. A
        # surface that ended in its own colours would draw a line under itself — the stripe
        # the body is built to avoid, moved one cell up.
        if crest + 1 < 16:
            px[x, crest + 1] = WATER_LIGHT
        for step, chance, seed in ((2, 0.6, 320), (3, 0.25, 321)):
            if crest + step < 16 and h(x, crest + step, seed) < chance:
                px[x, crest + step] = WATER_LIGHT
    return img


def lava() -> Image.Image:
    """The **plain** body of a lava pool: cell 0 of lava's texture.

    The same construction as [`water`] and for the same reason, with the opposite heat: the
    bright cells are the ones hot enough to glow, the dark ones are crust already cooled. A pool
    that read as a flat orange rectangle would be a tint; a pool with a crust is a material —
    and the crust belongs to [`lava_bed`], drawn once against the rock, rather than to every
    body cell, where a two-tile pool would stack two of them.
    """
    return lava_cells()[0].copy()


def lava_bed() -> Image.Image:
    """The bed of a lava pool: cooled rock, crusted where it meets the ground.

    The same overlay as [`water_bed`] — transparent above, all of it at the bottom, ragged in
    between — with one difference that is the whole character of the material: a few cracks
    in the crust still glow. The pool is hot enough that the rock it sits on is *cooking*,
    and a bed that was uniformly dark would say the bottom of a lava pool is cold.
    """
    img = cell()
    px = img.load()
    for x in range(16):
        for y in range(12, 16):
            if h(x, y, 371) > (0.2, 0.55, 0.85, 1.0)[y - 12]:
                continue
            crust = LAVA_CRUST if h(x, y, 372) < 0.7 else LAVA_DEEP
            if y == 15 and h(x, y, 373) > 0.88:
                crust = LAVA_MID
            px[x, y] = (*crust, 255 if y > 12 else 200)
    return img


def _fluid_wall(edge, shade, seed: int) -> Image.Image:
    """One side of a pool: a rim, drawn over the body cells of the pool's edge column.

    Uniform down the cell and **with no row structure at all**, for the reason the body has
    none: an edge runs the whole height of a pool, so its rim has to be one continuous line
    through every cell it crosses. Its top rows do not fade — a fade would put a gap in the
    line every sixteenth pixel — and the two or three pixels of overflow above a surface
    cell's crest read as the waterline climbing the wall, which is what a waterline does.

    Left only: the right wall is this picture mirrored, so the two cannot drift apart the way
    the turret's two directions did before they were one drawing (`crate::view`).
    """
    img = cell()
    px = img.load()
    for y in range(16):
        px[0, y] = (*edge, 255)
        if h(1, y, seed) < 0.75:
            px[1, y] = (*shade, 200)
        if h(2, y, seed + 1) < 0.3:
            px[2, y] = (*shade, 120)
    return img


def water_wall_l() -> Image.Image:
    """Water's rim against open air: dark, wet, and two or three pixels deep."""
    return _fluid_wall(WATER_DEEP, WATER_MID, 391)


def water_wall_r() -> Image.Image:
    return water_wall_l().transpose(Image.FLIP_LEFT_RIGHT)


def lava_wall_l() -> Image.Image:
    """Lava's rim against open air: the crust that forms where the melt meets nothing."""
    return _fluid_wall(LAVA_CRUST, LAVA_DEEP, 392)


def lava_wall_r() -> Image.Image:
    return lava_wall_l().transpose(Image.FLIP_LEFT_RIGHT)


def lava_top(frame: int) -> Image.Image:
    """A lava surface: a glowing crust line with two domes that break and re-form.

    The domes are why there are two frames: a pool that only changed colour would read as a
    light, and what a vent does is *bulge* just before it throws. Nothing here is the ball
    itself — the ball is a sprite of its own (`lava_ball`), thrown into the air above this
    line — but a surface that domes says the pool is doing something.
    """
    img = lava()
    px = img.load()
    # The hot rows fade into the body tone by the fourth row, for the reason `water_top`
    # gives: the cell below a surface is a body cell, and a surface that ended in its own
    # colours would draw a line under itself. The top row is the crust line itself — a
    # molten pool has a cooled skin, so the brightest line of the whole pool is the one the
    # body *does not* have.
    for x in range(16):
        px[x, 0] = LAVA_GLOW if h(x, 0, 341) < 0.25 else LAVA_HOT
        px[x, 1] = LAVA
        if h(x, 2, 342) < 0.55:
            px[x, 2] = LAVA
        if h(x, 3, 343) < 0.55:
            px[x, 3] = LAVA
    domes = (4, 11) if frame == 0 else (7, 13)
    for cx in domes:
        for dx in (-1, 0, 1):
            px[cx + dx, 0] = LAVA_GLOW
            px[cx + dx, 1] = LAVA_HOT
        px[cx, 2] = LAVA_HOT
    return img


def lava_ball() -> Image.Image:
    """A ball of lava in flight: a glowing core inside a cooling shell.

    Drawn **to fill its cell**, because the game draws this cell at the ball's own hitbox
    (`crate::view::draw_lava_ball`): the thing that kills a body has to be the thing the
    player can see, so the picture and the box are the same rectangle by construction
    rather than by a size constant that could drift from it.
    """
    img = cell()
    px = img.load()
    for y in range(16):
        for x in range(16):
            d2 = (x + 0.5 - 8.0) ** 2 + (y + 0.5 - 8.0) ** 2
            if d2 > 46:
                continue
            if d2 > 36:
                px[x, y] = LAVA_DEEP
            elif d2 > 22:
                px[x, y] = LAVA
            elif d2 > 10:
                px[x, y] = LAVA_HOT
            else:
                px[x, y] = LAVA_GLOW
    # Two sparks off the top, so a still frame of the ball reads as *thrown*.
    px[6, 1] = LAVA_HOT
    px[10, 1] = LAVA
    return img


def vine() -> Image.Image:
    """A vine: a stem through the whole cell with leaves either side.

    **The stem is continuous from row 0 to row 15 and does not wander**, because a vine is
    the one tile in the level that is *stacked*: any sway in the stem is a kink at every
    tile boundary, so a shaft of ten of these reads as ten cuttings. The leaves alternate
    sides down the cell, which is where the organic shape comes from instead.

    The stem bottom is a darker knot, which is what a vine's own weight does to it.
    """
    img = cell()
    px = img.load()
    for y in range(16):
        px[7, y] = VINE_STEM
        px[8, y] = VINE
        px[9, y] = VINE_DARK
    for index, y in enumerate((2, 6, 10, 13)):
        right = index % 2 == 0
        bx = 10 if right else 6
        step = 1 if right else -1
        px[bx, y] = VINE_LIGHT
        px[bx + step, y] = VINE
        px[bx + step, y + 1] = VINE_DARK
        px[bx, y + 1] = VINE
    px[8, 15] = VINE_DARK
    return img


def rope() -> Image.Image:
    """A rope hanging from the tile above: a twisted cord, a knot and a loop at its end.

    The palette's picture of the rope brush, and the only *picture* of a rope in the game: in a
    level the view draws the line itself ([`crate::view::draw_ropes`]), because a rope's shape is
    the shape of its swing and a sprite is one pose. So this has to say "rope" at palette size and
    nothing else has to — which is why it is a twist, a knot and a loop rather than a straight
    strand. A plain vertical line reads as a wire, and the vine beside it in the palette is the
    other thing a hanging line in this tileset can be, with leaves this deliberately has none of.

    The three shades are the view's own (`ROPE_CORD`, `ROPE_LIGHT` and `ROPE_KNOT` are these wood
    tones): a brush whose icon is a different brown from the line it paints is two ropes.
    """
    img = cell()
    px = img.load()

    def put(x: int, y: int, colour) -> None:
        px[x, y] = colour

    # Tied under a ceiling: a bar of a knot across the top, and the strand leaving its underside.
    for x in range(5, 11):
        put(x, 0, WOOD_DARK)
    put(6, 0, WOOD_TOP)
    for x in range(6, 10):
        put(x, 1, WOOD_MID)
    # The twist: three pixels wide, with the highlight stepping from one side of the cord to the
    # other every three rows. That step is the whole of the rope's texture — a column of one shade
    # is a cable, and a two-row checker is noise at this size.
    step = [(8, 2), (8, 3), (7, 4), (7, 5), (8, 7), (8, 8), (7, 9), (7, 10)]
    for y in range(2, 11):
        for x in (7, 8):
            put(x, y, WOOD_MID)
        put(9, y, WOOD_DARK)
    for x, y in step:
        put(x, y, WOOD_LIGHT)
    # A knot half way down, then the loop its free end is turned into: the two things the view's
    # own drawing uses to stop a line reading as a wire, said again here at eight pixels wide.
    for x in range(6, 11):
        put(x, 6, WOOD_DARK)
    put(6, 6, WOOD_TOP)
    put(10, 6, WOOD_TOP)
    put(7, 6, WOOD_MID)
    for x in range(6, 10):
        put(x, 11, WOOD_DARK)
    put(6, 11, WOOD_LIGHT)
    for y in (12, 13, 14):
        put(6, y, WOOD_LIGHT)
        put(9, y, WOOD_DARK)
    for x in range(6, 10):
        put(x, 15, WOOD_DARK)
    put(7, 15, WOOD_MID)
    put(8, 15, WOOD_MID)
    put(7, 12, WOOD_DARK)
    put(8, 12, WOOD_DARK)
    return img


def rope_sprites() -> list[tuple[str, Image.Image]]:
    """The rope's own picture, as a family of one (see [`rope`] and `gen_tileset.FAMILIES`)."""
    return [("rope", rope())]


def spike_ball() -> Image.Image:
    """A spiked ball on a chain: an iron ball with eight spikes, drawn to fill its cell.

    The palette's picture of the spiked-ball brush, and the *ball's* own picture in a level: the
    chain is a line and the view draws it ([`crate::view::draw_spike_balls`]), but the ball is a
    body with a hitbox, and the thing that kills a player has to be the thing they can see. So the
    ball fills its cell the way [`lava_ball`] does — the sim's box is most of a tile and the art
    is that box — rather than sitting in the middle of one with air around it.

    Two things make it read as a *hazard* rather than as a rock: the spikes are a colour no
    material in this tileset uses for a surface (cold iron against warm stone), and the ball is
    shaded as a sphere, so the silhouette is a circle with teeth instead of a blob. The spikes
    point along the eight compass directions and are drawn *behind* the body, which is what makes
    them look attached rather than stuck on.
    """
    img = cell()
    px = img.load()
    body = 20.0
    for y in range(16):
        for x in range(16):
            dx = x + 0.5 - 8.0
            dy = y + 0.5 - 8.0
            d2 = dx * dx + dy * dy
            if d2 <= body:
                # Lit from the top left, dark along the rim away from it: the one shading trick
                # the rest of the tileset uses, so the ball belongs to the same light.
                if d2 > body * 0.6 and dy > 0.5:
                    px[x, y] = STONE_DARK
                elif d2 > body * 0.6 and dx + dy < 0.0:
                    px[x, y] = STONE_LIGHT
                elif dx + dy < -3.0:
                    px[x, y] = STONE_LIGHT
                else:
                    px[x, y] = STONE_MID
                continue
            # The eight spikes: a band one pixel either side of each axis and each diagonal,
            # running out to the cell's edge. The tip is darker than the root, which is what
            # keeps a spike from reading as a ray of light.
            on_spike = (
                abs(dx) <= 1.0
                or abs(dy) <= 1.0
                or abs(abs(dx) - abs(dy)) <= 1.8
            )
            if on_spike and d2 <= 50.0:
                px[x, y] = IRON_LIGHT if d2 <= 40.0 else IRON_BOLT
    # A rivet on the lit side says *forged*, and gives the eye a scale for the whole ball.
    px[5, 5] = IRON_LIGHT
    return img


def spike_ball_sprites() -> list[tuple[str, Image.Image]]:
    """The spiked ball's picture, as a family of one (see [`spike_ball`])."""
    return [("spike_ball", spike_ball())]


def sprites() -> list[tuple[str, Image.Image]]:
    """Every terrain sprite, in the order the manifest lists them."""
    out: list[tuple[str, Image.Image]] = []
    for mask in range(16):
        out.append((f"ground_{mask}", ground(mask)))
    for mask in DEEP_MASKS:
        out.append((f"deep_{mask}", deep(mask)))

    # The second material, in the same two halves: a beach's surface, and the sandstone
    # under it. A level paints them with one brush — the depth rule picks the half — so
    # the two families are one material and are listed together.
    for mask in range(16):
        out.append((f"sand_{mask}", sand(mask)))
    for mask in DEEP_MASKS:
        out.append((f"sandstone_{mask}", sandstone(mask)))

    # The third and fourth materials: brick over masonry, and wood over timber.
    for mask in range(16):
        out.append((f"brick_{mask}", brick(mask)))
    for mask in DEEP_MASKS:
        out.append((f"masonry_{mask}", masonry(mask)))
    for mask in range(16):
        out.append((f"wood_{mask}", wood(mask)))
    for mask in DEEP_MASKS:
        out.append((f"timber_{mask}", timber(mask)))

    # Variants: the tiles a level repeats most. The Rust view picks one by hashing
    # the tile coordinates, so a long floor is not one picture stamped sixty times.
    out.append(("ground_1_b", _with_detail(ground(1), _flowers)))
    out.append(("ground_1_c", _with_detail(ground(1), _tuft_and_pebble)))
    out.append(("deep_0_b", _with_detail(deep(0), _pebbles)))
    out.append(("deep_0_c", _with_detail(deep(0), _crack)))
    out.append(("sand_1_b", _with_detail(sand(1), _ripples)))
    out.append(("sand_1_c", _with_detail(sand(1), _shells)))
    out.append(("brick_1_b", _with_detail(brick(1), _brick_moss)))
    out.append(("brick_1_c", _with_detail(brick(1), _brick_cracked)))
    out.append(("wood_1_b", _with_detail(wood(1), _wood_knots)))
    out.append(("wood_1_c", _with_detail(wood(1), _wood_rivets)))

    out.append(("spike", spikes()))

    # The one-way shelf and the moving platform: one family of three each, because both
    # are a run of tiles and both need their two ends drawn.
    for part in ("l", "m", "r"):
        out.append((f"oneway_{part}", oneway(part)))
    for part in ("l", "m", "r"):
        out.append((f"mover_{part}", mover(part)))

    # The trap tiles, next to the spike because they are the same *kind* of thing: a tile the
    # physics has an opinion about, drawn as an object rather than as a material. The four cracks
    # go last of the group because the renderer draws them *over* the tile beneath, which is also
    # where they sit in a level.
    out.append(("spring", spring()))

    # The elements. Water and lava are a seamless body, a bed, and two surface frames each;
    # the ball and the vine are one cell each. They sit beside the spike and the pad because
    # they are the same *kind* of thing — a tile the physics has an opinion about rather than
    # a material. The bed is the sprite that lets the body be seamless: a pool's depth cue was
    # in the body once, and a two-tile pool drew it twice, which is the band this family was
    # reshaped to remove (`_fluid_texture`).
    # Bodies first, then the three cells a pool *ends* in. A fluid's autotile is built out of
    # those pieces rather than out of masks, the way a solid tile is: the body is seamless and
    # indexed by position (sixteen cells of one picture, `FLUID_BLOCK` square), and the
    # surface, the bed and the two walls are overlays or replacements that compose with
    # whichever body cell the position happened to pick. The plain `water`/`lava` cell is the
    # body's cell 0, and doubles as the fallback for a tileset that has no block.
    for name, cells in (("water", water_cells), ("lava", lava_cells)):
        for index, sprite in enumerate(cells()):
            out.append((f"{name}_b{index}", sprite))
    out.append(("water", water()))
    out.append(("water_top_a", water_top(0)))
    out.append(("water_top_b", water_top(1)))
    out.append(("water_bed", water_bed()))
    out.append(("water_wall_l", water_wall_l()))
    out.append(("water_wall_r", water_wall_r()))
    out.append(("lava", lava()))
    out.append(("lava_top_a", lava_top(0)))
    out.append(("lava_top_b", lava_top(1)))
    out.append(("lava_bed", lava_bed()))
    out.append(("lava_wall_l", lava_wall_l()))
    out.append(("lava_wall_r", lava_wall_r()))
    out.append(("lava_ball", lava_ball()))
    out.append(("vine", vine()))
    out.append(("cracked_ground", crack_overlay("ground")))
    out.append(("cracked_sand", crack_overlay("sand")))
    out.append(("cracked_brick", crack_overlay("brick")))
    out.append(("cracked_wood", crack_overlay("wood")))
    return out


def blends() -> list[tuple[str, Image.Image]]:
    """The material seams, in look order and then side order.

    A family of its own rather than the tail of [`sprites`], and it is the *last* family
    in the atlas for a reason worth keeping: the sprites above it keep the cell they have
    always had, so adding a seam is a purely additive diff to a generated file rather
    than a reshuffle of every sprite after it.
    """
    out: list[tuple[str, Image.Image]] = []
    for look in LOOKS:
        for side in SIDES:
            if side in BLIND_SIDES.get(look, ()):
                continue
            out.append((f"blend_{look}_{side}", blend(look, side)))
    return out
