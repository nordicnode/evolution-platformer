"""The scenery and the small effects: things that are drawn but never simulated.

Three groups, and each has a reason to be a sprite rather than a shape drawn in the
renderer:

* **Decor** — bushes, tufts of grass, flowers, rocks, pebbles, a mushroom. The level
  format has no concept of these, and that is deliberate: they are *dressing*, and the
  view scatters them deterministically along a solid tile's surface (hash of the tile
  coordinates, so a level looks the same every frame and a regenerated atlas does not
  move them). Dressing a floor with sprites is what stops a 60-tile corridor looking
  like one picture repeated, and a mushroom that lied about being solid would be a
  lie the player could learn from, so every one of these is drawn only where the
  physics agrees nothing is there.
* **Clouds** — the only part of the parallax that is art rather than a shape: a
  cumulus with a soft top and a heavier underside, in three shapes so the sky is not
  one cloud stamped across it.
* **Effects** — a dust puff for a runner's feet, a larger one for a landing, a burst
  for a death and a sparkle for the flag. They are sprites rather than shapes because
  they need to look like *air*, and alpha is the only way to say that.

Every sprite here has transparent padding and is positioned for the surface it sits
on: the ground-hugging props stand on row 15, because the view draws one at the
bottom of a solid tile. Deterministic throughout — the only randomness is [`h`], so
regenerating the atlas never produces a diff.
"""

from __future__ import annotations

from PIL import Image

from .common import (
    BARK,
    BARK_DARK,
    BARK_LIGHT,
    BUTTERFLY_BODY,
    BUTTERFLY_EDGE,
    BUTTERFLY_PALE,
    BUTTERFLY_PALE_DARK,
    BUTTERFLY_WING,
    BUTTERFLY_WING_DARK,
    CLOUD,
    CLOUD_DARK,
    CLOUD_LIGHT,
    DIRT,
    PINE,
    PINE_DARK,
    PINE_FAR,
    PINE_FAR_LIGHT,
    PINE_LIGHT,
    PINE_SHADE,
    TRANSPARENT,
    DIRT_DARK,
    DIRT_LIGHT,
    DIRT_MID,
    DUST,
    DUST_DARK,
    DUST_LIGHT,
    LEAF,
    LEAF_DARK,
    LEAF_DEEP,
    LEAF_LIGHT,
    MUSHROOM_CAP,
    MUSHROOM_LIGHT,
    MUSHROOM_STEM,
    PETAL,
    PETAL_DARK,
    PETAL_PINK,
    ROCK,
    ROCK_DARK,
    ROCK_LIGHT,
    SPARK,
    SPARK_WARM,
    TUFT,
    TUFT_DARK,
    TUFT_DRY,
    TUFT_DRY_DARK,
    cell,
    h,
)

# --- a drawing surface ------------------------------------------------------
#
# The helpers take the canvas size, because one sprite here is not a cell: a tree is
# drawn **whole, 16 wide and two cells tall**, and cut in half afterwards (see `tree`).
# The default is the cell every other sprite uses, so the calls that are about one cell
# do not have to say so.


def _put(px, x: int, y: int, colour, w: int = 16, hh: int = 16) -> None:
    if 0 <= x < w and 0 <= y < hh:
        px[x, y] = colour


def _box(px, x0: int, y0: int, x1: int, y1: int, colour, w: int = 16, hh: int = 16) -> None:
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            _put(px, x, y, colour, w, hh)


def _disc(px, cx: float, cy: float, r: float, colour, w: int = 16, hh: int = 16) -> None:
    """A filled circle, used for clouds, rocks and every puff of air."""
    for y in range(hh):
        for x in range(w):
            dx, dy = (x + 0.5) - cx, (y + 0.5) - cy
            if dx * dx + dy * dy <= r * r:
                _put(px, x, y, colour, w, hh)


def _lumpy_disc(
    px, cx: float, cy: float, r: float, colour, seed: int, w: int = 16, hh: int = 16
) -> None:
    """A disc with a noise-broken rim: an air puff, a cloud or a canopy, not a bubble."""
    for y in range(hh):
        for x in range(w):
            dx, dy = (x + 0.5) - cx, (y + 0.5) - cy
            d = (dx * dx + dy * dy) ** 0.5
            edge = r + h(x, y, seed) * 0.9 - 0.45
            if d <= edge:
                _put(px, x, y, colour, w, hh)


# --- decor ------------------------------------------------------------------


def bush(sway: int) -> Image.Image:
    """A leafy mound, one pixel of sway between frames and a few berries."""
    img = cell()
    px = img.load()
    _disc(px, 8.0, 11.5, 4.6, LEAF_DARK)
    _disc(px, 8.0 + sway * 0.6, 10.6, 3.8, LEAF)
    _disc(px, 6.2, 9.2, 2.4, LEAF_LIGHT)
    _disc(px, 9.6, 8.6, 2.0, LEAF_LIGHT)
    _box(px, 3, 15, 12, 15, LEAF_DARK)
    for x, y in ((5, 12), (10, 11), (8, 9)):
        _put(px, x, y, LEAF)
    for x in range(4, 12):
        if h(x, 14, 17) < 0.5:
            _put(px, x, 14, LEAF_DARK)
    if sway:
        _put(px, 12, 7, LEAF_LIGHT)
        _put(px, 13, 8, LEAF)
    return img


def tuft(sway: int) -> Image.Image:
    """Blades of grass rising off the ground."""
    img = cell()
    px = img.load()
    blades = ((4, 5), (6, 7), (8, 6), (10, 4), (12, 3))
    for x, height in blades:
        top = 15 - height
        for y in range(top, 16):
            _put(px, x, y, TUFT if y > top else TUFT_DARK)
        _put(px, x + (1 if sway else 0), top, TUFT_DARK)
    _box(px, 3, 15, 13, 15, TUFT_DARK)
    return img


def _flower(px, x: int, petal, centre) -> None:
    """One flower: a stem from the ground up to four petals and a middle."""
    _put(px, x, 11, LEAF_DARK)
    _put(px, x, 12, LEAF)
    _put(px, x, 13, LEAF)
    _put(px, x, 14, LEAF_DARK)
    _put(px, x - 1, 12, LEAF)
    for dx, dy in ((0, -1), (-1, 0), (1, 0), (0, 1), (0, 0)):
        _put(px, x + dx, 10 + dy, petal if (dx, dy) != (0, 0) else centre)


def flowers(petal, centre) -> Image.Image:
    """A small clump: three flowers at three heights, so it reads as a patch."""
    img = cell()
    px = img.load()
    for x in (4, 7, 10):
        _flower(px, x, petal, centre)
    _put(px, 5, 15, LEAF_DARK)
    _put(px, 9, 15, LEAF)
    return img


def rock(large: bool) -> Image.Image:
    """A boulder, or the pebble that goes with it."""
    img = cell()
    px = img.load()
    if large:
        _lumpy_disc(px, 8.0, 11.0, 4.4, ROCK_DARK, 71)
        _lumpy_disc(px, 8.0, 10.4, 3.8, ROCK, 73)
        _disc(px, 6.6, 9.0, 1.8, ROCK_LIGHT)
        _box(px, 4, 15, 12, 15, ROCK_DARK)
    else:
        _lumpy_disc(px, 7.0, 13.5, 2.2, ROCK_DARK, 79)
        _lumpy_disc(px, 7.0, 13.2, 1.7, ROCK, 83)
        # A pebble that stops a row short of the cell's floor reads as floating above the
        # ground it is drawn on, which is exactly what the terrain must never look like.
        _box(px, 5, 15, 9, 15, ROCK_DARK)
        _put(px, 6, 12, ROCK_LIGHT)
        _put(px, 11, 14, ROCK_DARK)
        _put(px, 10, 14, ROCK)
    return img


def mushroom() -> Image.Image:
    """A capped mushroom: it is dressing, and the player runs straight through it."""
    img = cell()
    px = img.load()
    _box(px, 6, 10, 9, 15, MUSHROOM_STEM)
    _box(px, 6, 10, 6, 15, MUSHROOM_STEM)
    _box(px, 9, 11, 9, 15, PETAL_DARK)
    _disc(px, 8.0, 8.0, 5.4, MUSHROOM_CAP)
    _box(px, 3, 8, 12, 8, MUSHROOM_CAP)
    _box(px, 4, 6, 11, 6, MUSHROOM_CAP)
    for x, y in ((5, 6), (8, 4), (10, 6), (7, 9)):
        _put(px, x, y, MUSHROOM_LIGHT)
    _box(px, 3, 10, 12, 10, PETAL_DARK)
    return img


def log() -> Image.Image:
    """A fallen log: a barrel of bark along the ground, sawn off at its right end."""
    img = cell()
    px = img.load()
    # The trunk: lit along the top, shaded underneath, so it reads as a cylinder.
    _box(px, 1, 12, 15, 15, DIRT_MID)
    _box(px, 1, 12, 15, 12, DIRT_LIGHT)
    _box(px, 1, 14, 15, 14, DIRT)
    _box(px, 1, 15, 15, 15, DIRT_DARK)
    # The near end is rounded off rather than squared.
    _put(px, 1, 12, DIRT)
    _put(px, 1, 13, DIRT_MID)

    # The sawn face, and the growth rings inside it: two rings is the whole difference
    # between a log and a plank.
    _box(px, 10, 11, 15, 15, DIRT_LIGHT)
    _box(px, 11, 12, 14, 15, DIRT)
    _box(px, 12, 12, 13, 15, DIRT_LIGHT)
    _box(px, 12, 13, 13, 14, DIRT)

    # Bark: a speckle, then moss along the top where it has been lying a while.
    for x in range(2, 10):
        if h(x, 13, 113) < 0.22:
            _put(px, x, 13, DIRT_DARK)
        if h(x, 14, 127) < 0.18:
            _put(px, x, 14, DIRT_MID)
    for x in (3, 4, 8):
        _put(px, x, 11, LEAF_DARK)
    _put(px, 9, 11, LEAF)
    return img


def stump() -> Image.Image:
    """A stump: roots, a trunk, a ringed cut top, and a shoot off one side."""
    img = cell()
    px = img.load()
    # Roots first, so it stands rather than hovers.
    _box(px, 2, 14, 13, 15, DIRT_MID)
    _box(px, 2, 15, 13, 15, DIRT_DARK)
    _box(px, 2, 14, 2, 15, DIRT)
    _box(px, 13, 14, 13, 15, DIRT)

    # The trunk, with its shadow on the side the sun is not.
    _box(px, 4, 7, 11, 14, DIRT)
    _box(px, 4, 7, 4, 14, DIRT_MID)
    _box(px, 11, 7, 11, 14, DIRT_DARK)

    # The cut top, ringed.
    _box(px, 3, 5, 12, 8, DIRT_LIGHT)
    _box(px, 3, 8, 12, 8, DIRT_MID)
    _box(px, 6, 6, 9, 7, DIRT)
    _box(px, 7, 6, 8, 7, DIRT_LIGHT)
    _put(px, 3, 5, DIRT_MID)
    _put(px, 12, 5, DIRT_MID)

    # It is felled, not dead: a shoot out of the side, and moss at the base.
    _put(px, 12, 4, LEAF)
    _put(px, 13, 3, LEAF)
    _put(px, 13, 2, LEAF_LIGHT)
    _put(px, 5, 13, LEAF_DARK)
    _put(px, 6, 13, LEAF_DARK)
    _put(px, 4, 12, LEAF_DARK)
    return img


def reeds(sway: int) -> Image.Image:
    """Cattails: the tallest thing that grows out of the ground, in two frames of sway."""
    img = cell()
    px = img.load()
    for x, top in ((4, 4), (8, 1), (11, 6)):
        for y in range(top, 16):
            _put(px, x, y, TUFT_DARK)
        # The seed head: two rows of it, which is what says reed rather than stem.
        _put(px, x, top, DIRT_MID)
        _put(px, x, top + 1, DIRT_MID)
        _put(px, x, top + 2, DIRT_DARK)
        if sway:
            _put(px, x + 1, top, DIRT_MID)
            _put(px, x + 1, top + 1, DIRT_MID)
    # Blades, lower and greener, and leaning with the same air.
    for x, top in ((6, 9), (13, 10)):
        for y in range(top, 16):
            _put(px, x + (1 if sway else 0), y, TUFT)
    _box(px, 3, 15, 13, 15, TUFT_DARK)
    if sway:
        _put(px, 7, 8, TUFT)
        _put(px, 12, 7, TUFT)
    return img


def vine(sway: int) -> Image.Image:
    """A vine hanging from the tile above, in two frames of sway.

    Its row 0 is opaque on purpose: a vine whose stem began a pixel below the top of the
    cell would look like it was floating under the platform rather than growing out of it,
    which is the same rule the spike has at the other end of the tile.
    """
    img = cell()
    px = img.load()
    lean = 1 if sway else 0

    def stem_x(y: int) -> int:
        return 7 + (1 if y > 5 else 0) + (lean if y > 8 else 0)

    for y in range(0, 13):
        x = stem_x(y)
        _put(px, x, y, LEAF_DARK)
        if h(x, y, 131) < 0.45:
            _put(px, x + 1, y, LEAF_DARK)
    # Leaves alternating down the stem, the lower ones leaning as the tip does.
    for y, side in ((3, -1), (6, 1), (9, -1), (11, 1)):
        x = stem_x(y)
        _put(px, x + side, y, LEAF)
        _put(px, x + side * 2, y + 1, LEAF)
        _put(px, x + side, y + 1, LEAF_LIGHT)
    return img


def mushroom_pair() -> Image.Image:
    """Two mushrooms rather than one: they grow in patches, and one tile of one mushroom
    reads as a sticker."""
    img = cell()
    px = img.load()

    _box(px, 3, 11, 5, 15, MUSHROOM_STEM)
    _box(px, 5, 12, 5, 15, PETAL_DARK)
    _disc(px, 4.5, 10.0, 3.2, MUSHROOM_CAP)
    _box(px, 2, 10, 7, 10, MUSHROOM_CAP)
    _put(px, 3, 9, MUSHROOM_LIGHT)
    _put(px, 6, 10, MUSHROOM_LIGHT)

    _box(px, 9, 9, 12, 15, MUSHROOM_STEM)
    _box(px, 12, 10, 12, 15, PETAL_DARK)
    _disc(px, 10.5, 8.0, 3.9, MUSHROOM_CAP)
    _box(px, 8, 8, 13, 8, MUSHROOM_CAP)
    _put(px, 9, 6, MUSHROOM_LIGHT)
    _put(px, 12, 8, MUSHROOM_LIGHT)
    _put(px, 10, 10, MUSHROOM_LIGHT)
    _box(px, 8, 10, 13, 10, PETAL_DARK)
    return img


def berry_bush(sway: int) -> Image.Image:
    """A taller bush with berries on it: the shape that says autumn rather than green.

    Drawn in the same tones as the other bushes because a bush with different foliage
    from the bush beside it is two plants, not two *bushes* — the red is what makes it a
    different species, and red fruit in a green mound is also the one thing on a lawn
    that a player's eye will find without being told to.
    """
    img = cell()
    px = img.load()
    _disc(px, 8.0, 11.0, 4.8, LEAF_DARK)
    _disc(px, 8.0 + sway * 0.5, 10.2, 4.0, LEAF)
    _disc(px, 6.0, 8.4, 2.4, LEAF_LIGHT)
    _disc(px, 10.4, 7.8, 2.0, LEAF_LIGHT)
    _box(px, 3, 15, 12, 15, LEAF_DARK)
    for x in range(4, 12):
        if h(x, 14, 19) < 0.5:
            _put(px, x, 14, LEAF_DARK)
    # Four berries, hanging on the near side where the light does not reach.
    for x, y in ((5, 12), (10, 11), (7, 9), (11, 13)):
        _put(px, x, y, MUSHROOM_CAP)
    _put(px, 6, 11, MUSHROOM_LIGHT)
    if sway:
        _put(px, 12, 8, LEAF_LIGHT)
        _put(px, 13, 9, LEAF)
    return img


def low_bush(sway: int) -> Image.Image:
    """A wide, low bush: half the height of the others and twice as broad.

    The shape exists because a floor dressed out of one bush is a floor of one bush. It
    is also the bush that survives a corridor: at half the height it still fits under a
    one-tile ceiling, where the rounded ones are refused for want of room.
    """
    img = cell()
    px = img.load()
    _disc(px, 5.0, 13.0, 3.6, LEAF_DARK)
    _disc(px, 11.0, 13.2, 3.4, LEAF_DARK)
    _disc(px, 7.6, 13.8, 2.6, LEAF_DARK)
    _disc(px, 5.0 + sway * 0.4, 12.2, 3.0, LEAF)
    _disc(px, 11.0 + sway * 0.4, 12.4, 2.8, LEAF)
    _disc(px, 4.2, 11.0, 1.6, LEAF_LIGHT)
    _disc(px, 9.8, 10.8, 1.6, LEAF_LIGHT)
    _box(px, 2, 15, 13, 15, LEAF_DARK)
    for x in range(3, 13):
        if h(x, 14, 29) < 0.45:
            _put(px, x, 14, LEAF_DARK)
    return img


def dry_tuft(sway: int) -> Image.Image:
    """Long blades that have gone to straw: the grass family's third shape.

    The one that is not green. A meadow drawn from one green tuft repeated is a lawn,
    and this is what keeps a field of grass from being a texture — it is also the tone
    the reeds already use, so the dry grass and the reeds beside it are one season.
    """
    img = cell()
    px = img.load()
    for x, height in ((3, 8), (5, 11), (7, 9), (9, 12), (11, 7), (13, 5)):
        top = 15 - height
        lean = 1 if sway and height > 9 else 0
        for y in range(top, 16):
            _put(px, x + lean, y, TUFT_DRY_DARK if y <= top + 1 else TUFT_DRY)
        _put(px, x + lean, top, TUFT_DRY_DARK)
    _box(px, 2, 15, 13, 15, TUFT_DRY_DARK)
    return img


def turf(sway: int) -> Image.Image:
    """Short, dense turf: the lawn the other tufts are standing in.

    Where `tuft` is five blades and `dry_tuft` is six long ones, this is every column at
    once, a couple of pixels high. It is what a flat floor wants more of, and what makes
    the tall tufts read as *tall* instead of as the only grass there is.
    """
    img = cell()
    px = img.load()
    for x in range(1, 15):
        height = 2 + int(h(x, 15, 33) * 3)
        top = 15 - height
        for y in range(top, 16):
            _put(px, x, y, TUFT_DARK if y <= top + 1 else TUFT)
    for x, top in ((4, 10), (9, 9), (12, 11)):
        for y in range(top, 16):
            _put(px, x + (1 if sway else 0), y, TUFT_DARK if y <= top + 1 else TUFT)
    _box(px, 1, 15, 14, 15, TUFT_DARK)
    return img


def tree(shape: int) -> tuple[Image.Image, Image.Image]:
    """A tree, returned as `(the cell its trunk stands in, the cell above it)`.

    Drawn as one picture 16 wide and **two cells tall** and then cut in half, rather
    than drawn twice as two sprites. The two halves meet at a seam — the base's top row
    against the top's bottom row — and a seam that does not line up is a tree with a
    stripe of sky through its canopy; cutting one drawing guarantees they line up, and
    the atlas test says so anyway, because the next tree might be drawn as two.

    The trunk is painted first and the canopy over it, for the same reason `pine` does
    it that way: a trunk laid over the foliage is a dark bar across the tree, and what
    should be visible instead is the bare stem *under* the lowest branches and the bark
    inside the canopy's notch. Shape 0 is a broad one with a heavy round crown and a
    short trunk; shape 1 is taller and narrower, with its crown in tiers, so a wood is
    not one tree at two sizes.
    """
    W, H = 16, 32
    img = Image.new("RGBA", (W, H), TRANSPARENT)
    px = img.load()
    if shape == 0:
        trunk, trunk_top = (6, 8), 17
        crown = ((7.5, 10.0, 5.4), (4.6, 14.5, 4.0), (10.6, 13.5, 4.2), (7.5, 17.5, 4.6))
    else:
        trunk, trunk_top = (6, 7), 15
        crown = (
            (6.5, 6.0, 3.6),
            (6.5, 10.5, 4.4),
            (6.5, 14.5, 4.6),
            (9.0, 8.5, 3.0),
            (9.0, 13.0, 3.4),
        )
    # The trunk, roots included: a pole stuck into the ground is the failure mode here.
    _box(px, trunk[0], trunk_top, trunk[1], 31, BARK, W, H)
    _box(px, trunk[0], trunk_top, trunk[0], 31, BARK_DARK, W, H)
    if shape == 0:
        _box(px, trunk[0] - 1, 29, trunk[0] - 1, 31, BARK, W, H)
        _box(px, trunk[1] + 1, 29, trunk[1] + 1, 31, BARK, W, H)
    else:
        _box(px, trunk[0] - 1, 30, trunk[0] - 1, 31, BARK, W, H)
        _box(px, trunk[1] + 1, 30, trunk[1] + 1, 31, BARK, W, H)
    # The canopy, dark rim first and lit on the side the sun is: the same three-tone
    # construction the bushes use, at four times the size.
    for cx, cy, r in crown:
        _lumpy_disc(px, cx, cy, r, LEAF_DARK, 137, W, H)
    for cx, cy, r in crown:
        _lumpy_disc(px, cx, cy - 1.0, r * 0.86, LEAF, 139, W, H)
    for cx, cy, r in crown[1:]:
        _lumpy_disc(px, cx - 1.0, cy - 2.0, r * 0.5, LEAF_LIGHT, 149, W, H)
    # The underside: what a canopy four tiles wide must have, or it is a balloon.
    for x in range(2, 14):
        for y in range(20, 26):
            if px[x, y] == LEAF_DARK and h(x, y, 151) < 0.5:
                px[x, y] = LEAF_DEEP
    # And the seam, made to agree. Rows 15 and 16 are one row apart in the picture and one
    # *cell* apart on the map, and the canopy's rim is ragged per row, so the two disagree
    # by a pixel or two wherever the outline slopes: a column that is leaves above and sky
    # below is a one-pixel hole in the tree, at the boundary of every tree in the game.
    # Whatever the upper half reaches down to, the lower half takes.
    for x in range(W):
        if px[x, 15][3] > 0 and px[x, 16][3] == 0:
            px[x, 16] = px[x, 15]
    return img.crop((0, 16, 16, 32)), img.crop((0, 0, 16, 16))


def butterfly(shape: int, frame: int) -> Image.Image:
    """A butterfly in flight: two wings and a body, wings up in one frame and down in the
    other.

    The only sprite in the game that is not standing on anything — it drifts above the
    level in the view's ambient layer — so it is drawn *around the middle* of its cell
    rather than on the bottom row. A butterfly pinned to row 15 is a moth on the ground,
    and the whole point of the layer is that it is the one thing in a level that is
    airborne. Shape 0 is pale gold, shape 1 pale violet: both chosen to sit apart from
    the vermilion player and from the blue sky, which are the only two things a creature
    drifting over an open level is seen against.
    """
    img = cell()
    px = img.load()
    pale = shape == 1
    wing = BUTTERFLY_PALE if pale else BUTTERFLY_WING
    wing_dark = BUTTERFLY_PALE_DARK if pale else BUTTERFLY_WING_DARK
    # Four wings, not two. The first version of this sprite was a body with a wing above
    # it and a wing below, and at six pixels across that reads as a bug with legs: what
    # says *butterfly* is a fore pair and a hind pair on each side, so each side gets a
    # blob beside the shoulders and a smaller one beside the hips.
    open_fore = ((4, 6), (5, 6), (6, 6), (4, 7), (5, 7), (6, 7))
    open_hind = ((4, 9), (5, 9), (6, 9), (5, 10), (6, 10))
    # The second frame folds the tips in by a column: a wingbeat at this size is the
    # silhouette changing, because a pixel of travel is not visible.
    shut_fore = ((5, 6), (6, 6), (5, 7), (6, 7))
    shut_hind = ((5, 9), (6, 9), (5, 10))
    left = open_fore + open_hind if frame == 0 else shut_fore + shut_hind
    for x, y in left:
        _put(px, x, y, wing)
        _put(px, 15 - x, y, wing)
    # The leading edge, one pixel of rust along the outer column, so a pale wing has an
    # outline against a pale sky.
    edge = ((4, 6), (4, 7)) if frame == 0 else ((5, 6), (5, 7))
    for x, y in edge:
        _put(px, x, y, BUTTERFLY_EDGE)
        _put(px, 15 - x, y, BUTTERFLY_EDGE)
    if pale:
        _put(px, 5, 9, wing_dark)
        _put(px, 10, 9, wing_dark)
        _put(px, 5, 6, wing_dark)
        _put(px, 10, 6, wing_dark)
    # A body two pixels wide over three rows, and an antenna off each shoulder: a
    # one-pixel body is a gap between two wings.
    for y in (7, 8, 9):
        _put(px, 7, y, BUTTERFLY_BODY)
        _put(px, 8, y, BUTTERFLY_BODY)
    _put(px, 7, 6, BUTTERFLY_BODY)
    _put(px, 8, 6, BUTTERFLY_BODY)
    _put(px, 6, 5, BUTTERFLY_BODY)
    _put(px, 9, 5, BUTTERFLY_BODY)
    return img


def pine(shape: int, far: bool = False) -> Image.Image:
    """A conifer silhouette for the horizon, two shapes in two ranks.

    A silhouette because that is all a tree three hills away is: one dark mass with a
    ragged edge, and no interior detail the eye could resolve at this size. Its base is on
    row 15 like every other prop, so the view can plant it on the ridge of the hills it is
    drawn in front of rather than on the ground of a level.

    **The trunk goes down first and the skirts over it.** Painted the other way round, which
    is how this started, the trunk is a dark bar laid across the lower branches: the foliage
    hangs off the post instead of sitting on it, and the widest part of a tree is the part
    that looks glued on. What is left showing is the bare stem under the lowest skirt, which is
    the only part of a distant conifer's trunk an eye can see anyway — so the lowest tier is
    also five rows up rather than three, or the skirt would come down to the ground and the
    trunk would be one pixel of it.

    `far` draws the rank *behind* the tree-line, in the two-tone haze of `common.PINE_FAR_*`.
    The silhouette is the same one, because the difference between the two ranks is distance
    rather than species and a second species is invisible at this size — what changes is the
    palette, and how much of it there is: four tones of shading on the rank three hills away
    is detail nobody gets to see.
    """
    bar, shadow, lit, stem = (
        (PINE_FAR_LIGHT, PINE_FAR, PINE_FAR_LIGHT, PINE_FAR)
        if far
        else (PINE, PINE_DARK, PINE_LIGHT, PINE_SHADE)
    )
    img = cell()
    px = img.load()
    if shape == 0:
        # Tall and narrow: a fir. Five tiered skirts, each wider than the last.
        tiers = ((3, 3.0), (5, 4.4), (7, 5.4), (9, 6.0), (11, 5.2))
        trunk = (7, 8)
    else:
        # Squat and rounder: a spruce with the skirts closer together.
        tiers = ((4, 3.6), (6, 5.0), (8, 6.0), (10, 5.6))
        trunk = (7, 8)
    _box(px, trunk[0], 8, trunk[1], 15, stem)
    for y, half in tiers:
        span = int(half)
        _box(px, 8 - span, y, 7 + span, y, bar)
        _box(px, 8 - span, y, 8 - span + 1, y, lit)
        _put(px, 8 - span, y, shadow)
        _box(px, 8 - span, y + 1, 7 + span, y + 1, shadow)
    # A ragged edge rather than a comb: a few pixels of sky through the lowest skirt, and
    # never through the trunk.
    lowest = max(y for y, _ in tiers) + 1
    for x in range(2, 14):
        if h(x, lowest, 149) < 0.3 and px[x, lowest] == shadow:
            px[x, lowest] = TRANSPARENT
    return img


# --- clouds -----------------------------------------------------------------


def cloud(shape: int) -> Image.Image:
    """A cumulus: soft on top, heavier underneath, three different shapes."""
    img = cell()
    px = img.load()
    lumps = (
        ((5.0, 9.0, 3.4), (8.5, 7.5, 4.2), (11.5, 9.5, 3.0)),
        ((6.0, 10.0, 3.0), (9.0, 8.0, 3.6), (12.0, 10.5, 2.6)),
        ((4.5, 10.0, 2.6), (7.5, 8.5, 3.8), (11.0, 9.0, 3.4), (13.0, 11.0, 2.0)),
    )[shape % 3]
    for cx, cy, r in lumps:
        _lumpy_disc(px, cx, cy, r, CLOUD_DARK, 97)
    for cx, cy, r in lumps:
        _lumpy_disc(px, cx, cy - 1.2, r * 0.85, CLOUD, 101)
        _lumpy_disc(px, cx, cy - 2.2, r * 0.6, CLOUD_LIGHT, 103)
    return img


# --- effects ----------------------------------------------------------------


def dust(frame: int) -> Image.Image:
    """The puff a runner kicks up: two frames, the second lower and thinner."""
    img = cell()
    px = img.load()
    if frame == 0:
        _lumpy_disc(px, 5.0, 13.5, 2.6, DUST_DARK, 43)
        _lumpy_disc(px, 5.2, 13.2, 1.9, DUST, 47)
        _put(px, 4, 12, DUST_LIGHT)
        _put(px, 7, 12, DUST_LIGHT)
    else:
        _lumpy_disc(px, 4.5, 12.0, 3.0, DUST_DARK, 53)
        _lumpy_disc(px, 4.8, 11.8, 2.1, DUST, 59)
        _put(px, 3, 10, DUST_LIGHT)
        _put(px, 6, 9, DUST_LIGHT)
        _put(px, 8, 12, DUST_LIGHT)
    return img


def puff() -> Image.Image:
    """The burst an agent leaves behind when it dies."""
    img = cell()
    px = img.load()
    _lumpy_disc(px, 8.0, 9.0, 5.6, DUST_DARK, 61)
    _lumpy_disc(px, 8.0, 9.0, 4.4, DUST, 67)
    _lumpy_disc(px, 8.0, 8.4, 2.6, DUST_LIGHT, 71)
    for x, y in ((3, 4), (12, 4), (2, 11), (13, 11)):
        _put(px, x, y, DUST)
    return img


def spark(frame: int) -> Image.Image:
    """A four-point sparkle, the two frames rotated a little from each other."""
    img = cell()
    px = img.load()
    cx, cy = 8.0, 8.0
    arms = ((0, -1), (0, 1), (-1, 0), (1, 0)) if frame == 0 else ((0, -1), (0, 1), (-1, -1), (1, 1))
    for dx, dy in arms:
        for step in (2, 3, 4):
            x = int(cx + dx * step)
            y = int(cy + dy * step)
            _put(px, x, y, SPARK if step < 4 else SPARK_WARM)
    _put(px, 8, 8, SPARK)
    _put(px, 7, 7, SPARK_WARM)
    if frame == 0:
        _put(px, 5, 5, SPARK_WARM)
        _put(px, 10, 10, SPARK_WARM)
    return img


def sprites() -> list[tuple[str, Image.Image]]:
    """Every prop sprite, in the order the manifest lists them."""
    return [
        ("bush_a", bush(0)),
        ("bush_b", bush(1)),
        ("bush_c", berry_bush(0)),
        ("bush_d", berry_bush(1)),
        ("bush_e", low_bush(0)),
        ("bush_f", low_bush(1)),
        ("tuft_a", tuft(0)),
        ("tuft_b", tuft(1)),
        ("tuft_c", dry_tuft(0)),
        ("tuft_d", dry_tuft(1)),
        ("tuft_e", turf(0)),
        ("tuft_f", turf(1)),
        ("flowers", flowers(PETAL, PETAL_DARK)),
        ("flowers_pink", flowers(PETAL_PINK, PETAL_DARK)),
        ("rock", rock(large=True)),
        ("pebble", rock(large=False)),
        ("mushroom", mushroom()),
        # The second wave: wood and reeds for a grounded floor, vines for a ceiling, and
        # the two clumps the first set had only one of. Everything from here on is
        # *optional* art (see `crate::view::scenery_sprite`), so a tileset drawn before
        # these existed still runs with substitutes.
        ("flowers_white", flowers(CLOUD_LIGHT, PETAL)),
        ("log", log()),
        ("stump", stump()),
        ("reeds_a", reeds(0)),
        ("reeds_b", reeds(1)),
        ("vine_a", vine(0)),
        ("vine_b", vine(1)),
        ("mushroom_pair", mushroom_pair()),
        # The foreground wood: two trees, each drawn whole and cut into the two cells a
        # level gives it. Named `base` and `top` because that is what they are — the cell
        # the trunk stands in, and the cell above it — and the view refuses to plant one
        # where the second cell is not open.
        ("tree_a_base", tree(0)[0]),
        ("tree_a_top", tree(0)[1]),
        ("tree_b_base", tree(1)[0]),
        ("tree_b_top", tree(1)[1]),
        # Ambient life, in the view's own layer rather than in the scenery scatter: two
        # shapes, two frames of wingbeat each.
        ("butterfly_a_1", butterfly(0, 0)),
        ("butterfly_a_2", butterfly(0, 1)),
        ("butterfly_b_1", butterfly(1, 0)),
        ("butterfly_b_2", butterfly(1, 1)),
        # The tree-line's two ranks: the one standing on the ridge, and the hazier one
        # behind it that turns the row of trees into a forest edge.
        ("pine_a", pine(0)),
        ("pine_b", pine(1)),
        ("pine_c", pine(0, far=True)),
        ("pine_d", pine(1, far=True)),
        ("cloud_a", cloud(0)),
        ("cloud_b", cloud(1)),
        ("cloud_c", cloud(2)),
        ("dust_a", dust(0)),
        ("dust_b", dust(1)),
        ("puff", puff()),
        ("spark_a", spark(0)),
        ("spark_b", spark(1)),
    ]
