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

from PIL import Image

from .common import (
    BRICK,
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

    # The trap tiles, next to the spike because they are the same *kind* of thing: a tile the
    # physics has an opinion about, drawn as an object rather than as a material. The four cracks
    # go last of the group because the renderer draws them *over* the tile beneath, which is also
    # where they sit in a level.
    out.append(("spring", spring()))
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
