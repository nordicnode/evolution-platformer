"""The cast and the level's own marks: the cat, the enemies, the flag, spawn, butterflies.

Every figure here is built the same way: a body of three to five tones from `kit`'s
palette, lit from the upper left, with the *shaded* side carried by a darker tone of
the same material rather than by a ring of ink. That sel-out is what keeps a figure
separable from the sky without making it look like a sticker with a cut line around
it — the thing the first course-kit pass got wrong.

The player is a cat blob rather than a person, and the reason is this file's: at
sixteen pixels a limb is two pixels and a knee is a guess, so every humanoid pose was
an argument about anatomy nobody can see. A blob has one outline and an expression,
and its animation lives where a player actually reads it — the bounce of the body, the
swing of the tail, the paddle of the paws. A cycle whose frames are one drawing is the
most invisible broken art there is, so every cycle below is a table of the parts that
move, and none of its frames repeat.
"""

from __future__ import annotations

from PIL import Image, ImageDraw

from .kit import (
    CORAL,
    CREAM,
    GOLD,
    IRON,
    Ramp,
    STONE,
    WHITE,
    canvas,
    put,
    rect,
)

# The player's ramps. Ginger, because the backdrop is a blue sky over green hills and the
# cat is the one warm mass on the screen; cream for the socks and the tip of the tail; a
# pink nose and inner ear; a darker ginger for the tabby's stripes. Four materials and one
# ink is the whole vocabulary — the figure this replaced had six, and at sixteen pixels
# six is four too many to read.
CAT_FUR = Ramp("#FFDCA8", "#F6B971", "#DC8F4E", "#B26C38", "#844820")
CAT_CREAM = Ramp("#FFFDF6", "#FFF3DE", "#F6DEBB", "#D8B98F", "#AE8E67")
CAT_PINK = Ramp("#FFCFC4", "#FDAFA2", "#F08C82", "#CE6A64", "#A64E50")
CAT_STRIPE = Ramp("#FBBF7E", "#E89A55", "#C87A3C", "#A05C2C", "#7A441F")
DARKISH = (54, 41, 47, 255)
WOODBASE = Ramp("#C39A6B", "#A87E52", "#8A6440", "#6B4B33", "#4E3626")

WALKER = Ramp("#C7B3F2", "#A48BDD", "#7F63BE", "#5F4796", "#443271")
CHASER = Ramp("#F6A48C", "#E8795F", "#CC5448", "#A33B3C", "#7A2B31")
FLYER = Ramp("#BDEBE0", "#8AD4C2", "#57AE9E", "#3C8A7C", "#28655C")

# --- The player -----------------------------------------------------------------------
#
# A cat, drawn as a ball. The figure this replaces was a person assembled from joint
# positions, and it was the wrong answer to the question — at sixteen pixels a limb is two
# pixels and a joint is a guess, so every pose was an argument about anatomy nobody can
# see. A ball has one outline; its animation lives where a player actually reads it: the
# bounce of the body, the swing of the tail, the paddle of the feet. A cycle whose frames
# are one drawing is the most invisible broken art there is, so every cycle below is a
# table of the parts that move, and none of its frames repeat.
#
# The silhouette is a table of spans, and `_spans` mirrors every one of them about the
# cell's centre line. That is not fussiness: the hand-drawn mask this replaces was a pixel
# or two wider on one side, and at this size a pixel is a whole cheek — which is what "his
# face looks skewed" means when you look at the sprite. Here a lopsided cat is not
# something to be careful about; it is unrepresentable.

# The ball: twelve wide, twelve tall, sitting one row above the floor so the feet have a
# row of their own. The crown is eight wide rather than twelve, because the ears need two
# pixels of head to grow out of at each corner; a ball that is already full width at the
# top has nowhere to put an ear except on top of it, which is how a cat becomes a bear.
CAT_BALL = (
    (3, 4, 11),
    (4, 2, 13),
    (5, 2, 13),
    (6, 2, 13),
    (7, 2, 13),
    (8, 3, 12),
    (9, 3, 12),
    (10, 4, 11),
    (11, 4, 11),
    (12, 5, 10),
    (13, 5, 10),
    (14, 6, 9),
)


# One ear: four rows, a one-pixel tip widening to five. Its base is flush with the crown
# and its widest row is the one *below* the crown, so the two are one silhouette with no
# step and no seam to shade. That is the whole trick: an ear that reaches even one pixel
# past the row under it reads as a hat brim, and one that stops short of the crown's
# corners reads as a horn stuck on afterwards.
CAT_EAR = (
    (0, 4, 4),
    (1, 3, 5),
    (2, 3, 6),
    (3, 2, 6),
)


def _spans(spans) -> set[tuple[int, int]]:
    """Every pixel in `spans`, mirrored left-to-right about the cell's centre line."""
    out: set[tuple[int, int]] = set()
    for y, x0, x1 in spans:
        for x in range(x0, x1 + 1):
            out.add((x, y))
            out.add((15 - x, y))
    return out


_CAT_MASK = _spans(CAT_BALL) | _spans(CAT_EAR)


def _cat_shade(img: Image.Image, mask: set[tuple[int, int]], dx: int, dy: int) -> None:
    """Fill `mask` at an offset, lit from the upper left.

    A rim of `lit` along the top and the left, a two-pixel band of `sh` down the bottom and
    the right — two pixels because one is an outline and two is a round — `deep` where the
    two shadowed sides meet, `mid` for everything inside. A diagonal gradient was tried
    here and it split the cat into a pale animal and an orange one; a rim keeps it one
    shape with an edge, which is how the enemies and the tiles are drawn as well.
    """
    for x, y in sorted(mask):
        top = (x, y - 1) not in mask
        left = (x - 1, y) not in mask
        right = 2 if (x + 1, y) not in mask else 1 if (x + 2, y) not in mask else 9
        bottom = 2 if (x, y + 1) not in mask else 1 if (x, y + 2) not in mask else 9
        # The highlight: the corner the light actually falls on, two pixels in from the
        # rim. A rim alone gives a shape one tone of roundness; a highlight gives it two,
        # and it is the only thing that separates a lit ball from a flat disc of fur.
        corner = (x - 3) + (y - 4)
        if (top or left) and corner < 4:
            tone = CAT_FUR.hi
        elif top or left:
            tone = CAT_FUR.lit
        elif right <= 2 or bottom <= 2:
            tone = CAT_FUR.deep if right == 2 and bottom == 2 else CAT_FUR.sh
        else:
            tone = CAT_FUR.mid
        put(img, x + dx, y + dy, tone)


def _cat_ears(img: Image.Image, dx: int, dy: int) -> None:
    """The inner ear: two pixels down the middle of each ear, in pink."""
    for x, y in ((4, 1), (5, 2)):
        put(img, x + dx, y + dy, CAT_PINK.mid)
        put(img, 15 - x + dx, y + dy, CAT_PINK.mid)


def _cat_markings(img: Image.Image, dx: int, dy: int) -> None:
    """The cat texture: a cream bib and a ginger stripe down each flank.

    This is the pattern, not a drawing of one. A tabby face at twelve pixels puts three
    dark marks within two rows of the eyes and then the sprite has four eyes — which is
    the exact failure this replaced — so the fur does its work on the *body*: a cream bib
    on the chest, and one warm stripe a side that follows the rim rather than crossing the
    face. Colour is what makes the ball read as a cat from across the screen; the ears and
    the nose are what make it read as one from close up.
    """
    for x, y, tone in (
        (6, 11, CAT_CREAM.mid),
        (7, 11, CAT_CREAM.lit),
        (8, 11, CAT_CREAM.lit),
        (9, 11, CAT_CREAM.mid),
        (7, 12, CAT_CREAM.mid),
        (8, 12, CAT_CREAM.mid),
        (7, 13, CAT_CREAM.sh),
        (8, 13, CAT_CREAM.sh),
    ):
        put(img, x + dx, y + dy, tone)
    for x, y in ((3, 10), (3, 11), (12, 10), (12, 11)):
        put(img, x + dx, y + dy, CAT_STRIPE.sh)


def _cat_tail(img: Image.Image, points: list[tuple[int, int]]) -> None:
    """The tail: a two-pixel curve through `points`, with a cream tip on the end.

    Two pixels rather than one because a one-pixel line against a sky is a scratch; the
    second column is the shadow side, so the tail has a round rather than a flat profile.
    """
    for i in range(len(points) - 1):
        x0, y0 = points[i]
        x1, y1 = points[i + 1]
        steps = max(abs(x1 - x0), abs(y1 - y0), 1)
        for j in range(steps + 1):
            t = j / steps
            x = round(x0 + (x1 - x0) * t)
            y = round(y0 + (y1 - y0) * t)
            put(img, x, y, CAT_FUR.mid)
            put(img, x + 1, y, CAT_FUR.sh)
    # The tip: one pixel, and cream. Two pixels of cream on a tail this short is a
    # marshmallow stuck to the cat's hip; one pixel reads as the end of an animal.
    x, y = points[-1]
    put(img, x, y, CAT_CREAM.lit)


def _cat_foot(img: Image.Image, x: int, y: int, near: bool = True) -> None:
    """A sock foot: three pixels along the floor, lit at the toe.

    One row tall, and drawn *behind* the ball rather than in front of it: a foot that is
    in the air is a foot behind the body, so a lifted step disappears up into the cat
    instead of sliding across its belly as a cream stamp. `near` picks the brighter of the
    two tones, which is all the depth a three-pixel foot needs.
    """
    tone = CAT_CREAM.mid if near else CAT_CREAM.sh
    for col in range(3):
        put(img, x + col, y, tone)
    put(img, x, y, CAT_CREAM.lit)


def _cat_face(
    img: Image.Image,
    dx: int,
    dy: int,
    blink: bool = False,
    wide: bool = False,
) -> None:
    """Two eyes and a nose, mirrored about the cell's centre line so the face cannot drift.

    Each eye is two by two, with one pixel of fur between them, which is the largest a
    face can be on a twelve-pixel ball without turning into a mask. There is no brow, no
    mouth and no whiskers: a face this size has room for three marks, and a fourth is a
    fourth thing that has to be balanced against the other three.
    """
    ey = 7
    if blink:
        # A shut eye is a line, not a dot: one pixel where the eye's lower half was.
        for x in (5, 10):
            put(img, x + dx, ey + dy + 1, CAT_FUR.deep)
            put(img, x + 1 + dx, ey + dy + 1, CAT_FUR.deep)
    else:
        height = 2 if wide else 2
        for x in (5, 9):
            rect(img, x + dx, ey + dy, x + 1 + dx, ey + dy + height - 1, DARKISH)
    # The nose: two pixels, centred, with the shadowed one to the right.
    put(img, 7 + dx, 10 + dy, CAT_PINK.mid)
    put(img, 8 + dx, 10 + dy, CAT_PINK.sh)


def _cat(
    bob: int = 0,
    lean: int = 0,
    tail: list[tuple[int, int]] | None = None,
    paws: tuple[tuple[int, int], tuple[int, int]] | None = None,
    blink: bool = False,
    wide: bool = False,
) -> Image.Image:
    """Assemble one frame: tail, feet, body, stripes, ears, face.

    `bob` moves the body, the ears, the face and the stripes; the feet are placed from the
    floor instead, because a planted foot that bobs with the body is a foot sliding on the
    ground. `paws` is (near, far), each (dx, lift): lift 0 is planted, and a lifted foot
    also tucks toward the middle, which puts it behind the body rather than across the
    belly — so a jump shows feet and a tucked jump shows none.
    """
    img = canvas()
    if tail:
        _cat_tail(img, [(x + lean, y + bob) for x, y in tail])
    if paws:
        (ndx, nlift), (fdx, flift) = paws
        _cat_foot(img, 4 + fdx, 15 - flift, near=False)
        _cat_foot(img, 9 + ndx, 15 - nlift, near=True)
    _cat_shade(img, _CAT_MASK, lean, bob)
    _cat_ears(img, lean, bob)
    _cat_markings(img, lean, bob)
    _cat_face(img, lean, bob, blink=blink, wide=wide)
    return img


# --- The poses ------------------------------------------------------------------------
#
# Every cycle below is a table of the three things that move: the bob of the body, the
# tail's curve and the two paws. The face is the same face in all of them, which is the
# whole point of drawing a blob — there is one character on screen and it is the same
# character in every frame.

# Standing: a breath, a slow blink, and the tail drifting. Six entries, and no two the
# same, because a cycle of one drawing is the most invisible broken art there is.
_IDLE_BOB = (0, 0, 0, -1, -1, 0)
_IDLE_SWAY = (0, 1, 2, 2, 1, 0)
_IDLE_TAIL = [(4, 13), (2, 13), (1, 12), (1, 11)]


def _cat_idle(frame: int) -> Image.Image:
    sway = _IDLE_SWAY[frame]
    tail = list(_IDLE_TAIL)
    tail[-1] = (tail[-1][0] + sway, tail[-1][1] - sway)
    tail[-2] = (tail[-2][0], tail[-2][1] - sway)
    return _cat(
        bob=_IDLE_BOB[frame],
        tail=tail,
        paws=[(0, 0), (0, 0)],
        blink=frame == 5,
    )


# The run: a gallop. The body bounces twice a cycle, the paws paddle half a cycle apart,
# and the tail streams out behind and waves — three rhythms on one body, which is what
# makes a sixteen-pixel animal look alive rather than translated.
_RUN_BOB = (0, 0, -1, -1, -1, 0, 0, 0, -1, -1, -1, 0)
_RUN_WAVE = (0, 0, -1, -2, -1, 0, 0, 0, -1, -2, -1, 0)
_RUN_TAIL = [(3, 12), (1, 12), (0, 11), (0, 10), (0, 9)]
# (dx, lift) for the near paw, by frame; the far paw reads the same table half a cycle on.
_RUN_PAW = (
    (1, 0),
    (1, 0),
    (1, 0),
    (0, 0),
    (-1, 0),
    (-1, 0),
    (-1, 0),
    (-1, 1),
    (-1, 1),
    (0, 1),
    (1, 1),
    (1, 1),
)


def _cat_run(frame: int) -> Image.Image:
    wave = _RUN_WAVE[frame]
    tail = list(_RUN_TAIL)
    tail[-1] = (tail[-1][0], tail[-1][1] + wave)
    tail[-2] = (tail[-2][0], tail[-2][1] + wave // 2)
    return _cat(
        bob=_RUN_BOB[frame],
        lean=1,
        tail=tail,
        paws=[_RUN_PAW[frame], _RUN_PAW[(frame + 6) % 12]],
    )


# Going up: the body stretches, the paws tuck, and the tail hangs — the one pose in which
# a cat's tail is the lowest thing about it. The tail is a different curve each frame, so
# the six are six drawings and not one held.
_JUMP_BOB = (-1, -1, -1, -1, 0, 0)
_JUMP_TAIL = (
    [(3, 12), (1, 12), (0, 13), (0, 14), (1, 15)],
    [(3, 12), (1, 12), (0, 13), (1, 14), (2, 15)],
    [(3, 12), (1, 13), (0, 14), (0, 15), (0, 15)],
    [(3, 12), (1, 13), (0, 14), (1, 15), (1, 15)],
    [(3, 12), (1, 12), (0, 12), (0, 13), (1, 14)],
    [(3, 12), (1, 12), (0, 12), (0, 13), (0, 14)],
)
_JUMP_PAW = ((0, 1), (0, 1), (1, 1), (1, 1), (1, 0), (1, 0))


def _cat_jump(frame: int) -> Image.Image:
    return _cat(
        bob=_JUMP_BOB[frame],
        lean=1,
        tail=_JUMP_TAIL[frame],
        paws=[_JUMP_PAW[frame], (-1, 1)],
        wide=True,
    )


# Coming down: the tail lifts and the paws come out to meet the floor.
_FALL_BOB = (-1, -1, -1, 0, 0, 0)
_FALL_TAIL = (
    [(3, 12), (1, 12), (0, 10), (0, 9), (1, 8)],
    [(3, 12), (1, 12), (0, 10), (0, 8), (1, 7)],
    [(3, 12), (1, 11), (0, 10), (1, 9), (2, 8)],
    [(3, 12), (1, 11), (0, 9), (0, 8), (1, 7)],
    [(3, 12), (1, 12), (0, 10), (1, 9), (1, 8)],
    [(3, 12), (1, 12), (0, 11), (0, 10), (1, 9)],
)
_FALL_PAW = ((-1, 0), (-2, 0), (-2, 0), (0, 0), (1, 0), (2, 0))


def _cat_fall(frame: int) -> Image.Image:
    return _cat(
        bob=_FALL_BOB[frame],
        lean=1,
        tail=_FALL_TAIL[frame],
        paws=[_FALL_PAW[frame], (1, 0)],
        wide=True,
    )


# Landing: the impact, the settle, and standing up again.
_LAND_BOB = (0, 1, 1, 0)
_LAND_TAIL = (
    [(4, 13), (2, 13), (1, 13), (1, 12), (1, 11)],
    [(4, 13), (2, 13), (1, 12), (1, 11), (1, 10)],
    [(4, 13), (2, 13), (1, 12), (2, 11), (2, 10)],
    [(4, 13), (2, 13), (1, 12), (1, 11), (1, 10)],
)
_LAND_PAW = ((1, 0), (0, 0), (-1, 0), (0, 0))


def _cat_land(frame: int) -> Image.Image:
    return _cat(
        bob=_LAND_BOB[frame],
        tail=_LAND_TAIL[frame],
        paws=[_LAND_PAW[frame], (-1, 0)],
        blink=frame == 3,
    )


def _cat_hurt() -> Image.Image:
    """The pose a dead agent is left in: on its side, eyes shut, feet in the air.

    Laid out along the cell rather than down it, because a body on its side is the one pose
    a player reads as *dead* rather than as crouching — the silhouette changes axis, which
    is the strongest signal sixteen pixels can send. The four feet pointing at the sky are
    the joke the shape allows: a cat that has fallen over is undignified, and a death sprite
    may as well be charming about it.
    """
    img = canvas()
    # The body, flattened along the floor: a long low mound from the tail to the shoulder.
    for y in range(12, 16):
        for x in range(1, 12):
            put(img, x, y, CAT_FUR.mid)
    for x in range(2, 11):
        put(img, x, 12, CAT_FUR.lit)
        put(img, x, 15, CAT_FUR.sh)
    put(img, 1, 12, CAT_FUR.lit)
    # The head at the front: a size larger than the body it lies against, because a cat's
    # head is the biggest thing about it.
    for y in range(10, 16):
        for x in range(9, 16):
            put(img, x, y, CAT_FUR.mid)
    for x in range(10, 16):
        put(img, x, 10, CAT_FUR.lit)
        put(img, x, 15, CAT_FUR.sh)
    put(img, 9, 11, CAT_FUR.lit)
    put(img, 9, 12, CAT_FUR.lit)
    # The ears, knocked flat against the head.
    for x, y in ((10, 9), (11, 9), (12, 9), (14, 9), (15, 9)):
        put(img, x, y, CAT_FUR.sh)
    # The eyes, shut: one line each, which is the calmest way to say a body has stopped.
    rect(img, 10, 12, 11, 12, DARKISH)
    rect(img, 13, 12, 14, 12, DARKISH)
    # The muzzle on the floor, so the nose points sideways.
    rect(img, 13, 14, 15, 14, CAT_CREAM.mid)
    put(img, 15, 13, CAT_PINK.mid)
    put(img, 15, 14, CAT_CREAM.sh)
    # The feet, up in the air.
    for x in (3, 6):
        rect(img, x, 10, x + 1, 11, CAT_CREAM.mid)
        put(img, x, 10, CAT_CREAM.lit)
        put(img, x + 1, 10, CAT_CREAM.lit)
    # The tail limp along the floor behind.
    for x in range(0, 3):
        put(img, x, 13, CAT_FUR.mid)
        put(img, x, 14, CAT_FUR.sh)
    put(img, 0, 13, CAT_CREAM.mid)
    return img


def player(pose: str, frame: int = 0) -> Image.Image:
    """One frame of the player: the cat, in whichever pose `pose` names."""
    if pose == "idle":
        return _cat_idle(frame % 6)
    if pose == "run":
        return _cat_run(frame % 12)
    if pose == "jump":
        return _cat_jump(frame % 6)
    if pose == "fall":
        return _cat_fall(frame % 6)
    if pose == "hurt":
        return _cat_hurt()
    return _cat_land(frame % 4)


# --------------------------------------------------------------------------------------
# Enemies
# --------------------------------------------------------------------------------------


def _beetle(img: Image.Image, ramp: Ramp, top: int, bottom: int, left: int, right: int,
            bob: int = 0) -> None:
    """A segmented carapace, lit from above-left: a body, not a dome sticker.

    The spans are explicit so the silhouette is a *drawing* — an ellipse-generated
    blob has no shoulder and no flat belly, and at 16 pixels those two read as a
    creature. The elytra split down the middle and the two side grooves give the
    shell plates; every tone is the creature's own ramp, so nothing is outlined.
    """
    spans = [
        (top + 0, left + 3, right - 2),
        (top + 1, left + 1, right - 1),
        (top + 2, left, right),
        (top + 3, left, right),
        (top + 4, left, right),
        (top + 5, left, right - 1),
        (top + 6, left + 1, right - 2),
        (top + 7, left + 2, right - 3),
    ][: bottom - top + 1]
    for row, (y, x0, x1) in enumerate(spans):
        y += bob
        last = row == len(spans) - 1
        for x in range(x0, x1 + 1):
            if row == 0:
                tone = ramp.hi if x0 + 1 <= x <= x1 - 1 else ramp.lit
            elif row == 1:
                tone = ramp.hi if x <= x0 + 2 else ramp.lit if x <= x1 - 2 else ramp.mid
            elif last:
                tone = ramp.deep if x > x0 + 1 else ramp.sh
            elif x == x0:
                tone = ramp.lit
            elif x == x1:
                tone = ramp.sh
            elif x >= x1 - 1:
                tone = ramp.sh
            elif row == 2 and x <= x0 + 2:
                tone = ramp.lit
            else:
                tone = ramp.mid
            put(img, x, y, tone)
    # The split between the wing cases, with a lit shoulder on its left: the one line
    # that makes a shell read as two plates rather than as a painted oval.
    seam = (left + right) // 2
    for row in range(1, len(spans) - 1):
        y = top + row + bob
        put(img, seam, y, ramp.deep)
        put(img, seam - 1, y, ramp.sh)
    # Side grooves, short enough that they are plates rather than stripes.
    for y in (top + 4 + bob, top + 5 + bob):
        put(img, left + 3, y, ramp.sh)
        put(img, right - 3, y, ramp.sh)


def _beetle_legs(img: Image.Image, ramp: Ramp, body_top: int, hips, moves, lifts) -> None:
    """Four legs from under the shell to the floor, one gait per frame.

    Each leg is drawn as a hip, a diagonal shin and a foot — three pixels that stay
    connected, because four loose dots under a body read as debris rather than as a
    walk. The foot's sideways offset and its lift are what the four frames change.
    """
    for x, move, lift in zip(hips, moves, lifts):
        put(img, x, body_top, ramp.sh)
        put(img, x, body_top + 1, ramp.deep)
        mid = x + (1 if move > 0 else -1 if move < 0 else 0)
        put(img, mid, body_top + 2, ramp.deep)
        put(img, x + move, 15 - lift, ramp.sh if lift == 0 else ramp.mid)


def walker(frame: int) -> Image.Image:
    img = canvas()
    # Four gaits. The body rides a pixel on the contact frames, so the silhouette
    # itself moves: a walk cycle whose torso is frozen is a sliding object.
    frame %= 4
    bob = (0, 1, 0, 1)[frame]
    moves = ((1, 1, -1, -1), (2, 0, -2, 0), (-1, -1, 1, 1), (0, 2, 0, -2))[frame]
    lifts = ((0, 0, 0, 0), (0, 1, 0, 1), (0, 0, 0, 0), (1, 0, 1, 0))[frame]
    _beetle_legs(img, WALKER, 12 + bob, (2, 5, 9, 12), moves, lifts)
    _beetle(img, WALKER, 4, 12, 1, 13, bob=bob)
    # A beetle's face, on the leading edge: a pale compound eye with a dark pupil and
    # a mouth plate under it. Two pixels of white is what reads at one tile.
    put(img, 10, 7 + bob, WHITE)
    put(img, 11, 7 + bob, WHITE)
    put(img, 11, 8 + bob, DARKISH)
    put(img, 10, 8 + bob, WALKER.hi)
    put(img, 12, 9 + bob, WALKER.deep)
    put(img, 12, 10 + bob, WALKER.sh)
    # An antenna, angled forward: the second shape that says insect.
    put(img, 12, 5 + bob, WALKER.lit)
    put(img, 13, 4 + bob, WALKER.mid)
    put(img, 14, 4 + bob, WALKER.sh)
    return img


def walker_dead() -> Image.Image:
    img = canvas()
    # A collapsed shell: still a shell, with the legs gone out from under it.
    rect(img, 2, 12, 12, 14, WALKER.mid)
    rect(img, 2, 12, 12, 12, WALKER.lit)
    rect(img, 2, 14, 12, 14, WALKER.deep)
    rect(img, 5, 10, 9, 11, WALKER.sh)
    put(img, 4, 11, WALKER.mid)
    put(img, 10, 11, WALKER.mid)
    put(img, 7, 13, WALKER.deep)
    put(img, 8, 13, WALKER.deep)
    # X eyes, splayed feet.
    for x, y in ((4, 12), (6, 14), (9, 12), (11, 14)):
        put(img, x, y, WALKER.deep)
    for x, y in ((5, 12), (4, 14), (10, 12), (12, 14)):
        put(img, x, y, DARKISH)
    for x in (3, 6, 11):
        put(img, x, 15, WALKER.sh)
    put(img, 8, 11, WALKER.lit)
    return img


def chaser(frame: int) -> Image.Image:
    img = canvas()
    # Lower, longer and faster than the walker: the red carapace sits nearer the
    # ground and the horn reaches forward over the leading eye.
    frame %= 4
    bob = (0, 1, 0, 1)[frame]
    moves = ((2, 0, -2, 0), (1, 1, -1, -1), (0, 2, 0, -2), (-1, -1, 1, 1))[frame]
    lifts = ((0, 1, 0, 1), (0, 0, 0, 0), (1, 0, 1, 0), (0, 0, 0, 0))[frame]
    _beetle_legs(img, CHASER, 12 + bob, (2, 5, 9, 12), moves, lifts)
    _beetle(img, CHASER, 6, 12, 0, 12, bob=bob)
    # The horn, stepped forward and up from the shell's brow, lit on its top edge.
    for i, (x, y) in enumerate(((13, 7), (14, 6), (15, 5))):
        put(img, x, y + bob, CHASER.hi if i == 2 else CHASER.lit)
        put(img, x, y + 1 + bob, CHASER.sh)
    put(img, 12, 7 + bob, CHASER.mid)
    put(img, 13, 8 + bob, CHASER.sh)
    # The eye: amber, narrow, and looking forward.
    rect(img, 9, 8 + bob, 11, 8 + bob, GOLD.lit)
    put(img, 10, 9 + bob, GOLD.mid)
    put(img, 11, 8 + bob, DARKISH)
    put(img, 9, 9 + bob, CHASER.sh)
    # Mandible.
    put(img, 12, 10 + bob, CHASER.deep)
    put(img, 13, 9 + bob, CHASER.deep)
    # Ridge bumps along the back, small enough to read as texture rather than horns.
    put(img, 3, 5 + bob, CHASER.sh)
    put(img, 6, 5 + bob, CHASER.sh)
    put(img, 9, 6 + bob, CHASER.sh)
    return img


def chaser_dead() -> Image.Image:
    img = canvas()
    rect(img, 1, 12, 13, 14, CHASER.mid)
    rect(img, 1, 12, 13, 12, CHASER.lit)
    rect(img, 1, 14, 13, 14, CHASER.deep)
    rect(img, 4, 10, 9, 11, CHASER.sh)
    put(img, 11, 10, CHASER.sh)
    for x in (2, 5, 9, 12):
        put(img, x, 15, CHASER.deep)
    put(img, 4, 13, DARKISH)
    put(img, 7, 13, DARKISH)
    put(img, 10, 11, DARKISH)
    put(img, 9, 11, CHASER.deep)
    return img


def flyer(frame: int) -> Image.Image:
    img = canvas()
    up = frame % 2 == 0
    # Wings: filled coral fans, raised on one frame and swept back on the other.
    for side in (-1, 1):
        for i in range(5):
            if up:
                x = 7 + side * (2 + i)
                y = 3 + i // 2
                for j in range(2):
                    put(img, x, y + j, CORAL.lit if j == 0 else CORAL.mid)
                put(img, x, y + 2, CORAL.sh)
            else:
                x = 7 + side * (2 + i)
                y = 5 + i // 3
                put(img, x, y, CORAL.lit)
                put(img, x, y + 1, CORAL.mid)
                put(img, x, y + 2, CORAL.sh)
    # Body.
    rect(img, 6, 5, 10, 11, FLYER.mid)
    rect(img, 6, 5, 6, 11, FLYER.lit)
    rect(img, 10, 5, 10, 11, FLYER.sh)
    rect(img, 6, 11, 10, 11, FLYER.deep)
    rect(img, 5, 6, 5, 9, FLYER.sh)
    rect(img, 8, 4, 10, 4, FLYER.lit)
    # Eyes.
    put(img, 8, 7, WHITE)
    put(img, 9, 7, WHITE)
    put(img, 8, 8, DARKISH)
    put(img, 9, 8, DARKISH)
    put(img, 7, 3, FLYER.deep)
    put(img, 9, 3, FLYER.deep)
    return img


def turret() -> Image.Image:
    """A stone plinth with a bronze dome and a barrel; the view mirrors it left."""
    img = canvas()
    rect(img, 2, 12, 13, 15, STONE.sh)
    rect(img, 2, 12, 13, 12, STONE.lit)
    rect(img, 2, 15, 13, 15, STONE.deep)
    for x in (4, 7, 10):
        put(img, x, 14, STONE.deep)
    draw = ImageDraw.Draw(img)
    draw.ellipse((3, 6, 12, 14), fill=STONE.mid)
    draw.ellipse((4, 7, 9, 12), fill=STONE.lit)
    put(img, 4, 8, STONE.hi)
    rect(img, 3, 11, 12, 12, STONE.sh)
    rect(img, 9, 7, 14, 9, IRON.mid)
    rect(img, 9, 7, 14, 7, IRON.hi)
    rect(img, 9, 9, 14, 9, IRON.sh)
    rect(img, 13, 7, 14, 9, IRON.mid)
    put(img, 14, 7, GOLD.hi)
    put(img, 14, 8, GOLD.mid)
    put(img, 14, 9, GOLD.sh)
    put(img, 12, 8, GOLD.lit)
    return img


def shot() -> Image.Image:
    """A turret's bolt: a hot core with a short tail, so a still frame reads as motion."""
    img = canvas()
    for x in range(2, 10):
        put(img, x, 8, GOLD.sh)
        if x > 4:
            put(img, x, 7, GOLD.mid)
            put(img, x, 9, GOLD.mid)
    rect(img, 10, 6, 14, 10, GOLD.mid)
    rect(img, 11, 6, 14, 9, GOLD.lit)
    rect(img, 12, 7, 14, 8, GOLD.hi)
    put(img, 15, 8, WHITE)
    put(img, 9, 8, GOLD.hi)
    return img


def goal_top() -> Image.Image:
    img = canvas()
    rect(img, 4, 0, 5, 15, CREAM.mid)
    rect(img, 4, 0, 4, 15, CREAM.hi)
    rect(img, 5, 0, 5, 15, CREAM.sh)
    rect(img, 3, 0, 6, 1, GOLD.mid)
    rect(img, 3, 0, 6, 0, GOLD.hi)
    put(img, 6, 1, GOLD.sh)
    return img


def goal_bottom() -> Image.Image:
    img = canvas()
    rect(img, 4, 0, 5, 13, CREAM.mid)
    rect(img, 4, 0, 4, 13, CREAM.hi)
    rect(img, 5, 0, 5, 13, CREAM.sh)
    rect(img, 2, 13, 7, 15, WOODBASE.mid)
    rect(img, 2, 13, 7, 13, WOODBASE.lit)
    rect(img, 2, 15, 7, 15, WOODBASE.deep)
    return img


def goal_flag(frame: int) -> Image.Image:
    """The flag, waving through four frames, hanging to the right of the pole."""
    img = canvas()
    # Four phases: the top edge lifts, then the bottom, then both settle the other way.
    # A symmetric `wave // 2` made frames one and three the same cloth.
    top_lift, bottom_lift = ((0, 0), (1, 0), (1, 1), (0, 1))[frame % 4]
    for x in range(6, 16):
        # The cloth's top and bottom edges travel in a wave, and the far edge lags.
        lag = (x - 6) // 4
        top = 1 + lag + top_lift
        bottom = 7 + lag - bottom_lift
        for y in range(top, bottom + 1):
            if y == top:
                tone = CORAL.hi
            elif y >= bottom - 1:
                tone = CORAL.sh
            elif y == top + 2:
                tone = CORAL.lit
            else:
                tone = CORAL.mid
            put(img, x, y, tone)
    for i in range(3):
        put(img, 9 + i, 3 + i, CORAL.sh)
    rect(img, 6, 1 + top_lift, 7, 7 - bottom_lift, GOLD.mid)
    put(img, 6, 1 + top_lift, GOLD.hi)
    return img


def spawn_marker() -> Image.Image:
    """Where the runner starts, editor only: a soft gold ring with a chevron."""
    img = canvas()
    draw = ImageDraw.Draw(img)
    draw.ellipse((1, 1, 14, 14), outline=GOLD.mid, width=2)
    draw.ellipse((3, 3, 12, 12), outline=GOLD.sh, width=1)
    for i in range(3):
        put(img, 6 + i, 8 - i, GOLD.hi)
        put(img, 6 + i, 8 + i, GOLD.hi)
        put(img, 9 + i, 8 - i, GOLD.mid)
        put(img, 9 + i, 8 + i, GOLD.mid)
    return img


def butterfly(frame: int, kind: int) -> Image.Image:
    """Two butterflies, two wingbeats each, drawn around the middle of the cell."""
    img = canvas()
    up = frame % 2 == 0
    wing = GOLD if kind == 0 else PURPLE_WING
    accent = CREAM if kind == 0 else GOLD
    spread = 3 if up else 2
    rect(img, 7, 6, 8, 11, (74, 58, 66, 255))
    for side in (-1, 1):
        for i in range(3):
            x = 7 + side * (spread + i)
            y = 6 + i
            put(img, x, y, wing.hi if i == 0 else wing.lit)
            put(img, x, y + 1, wing.mid)
            put(img, x, y + 2, wing.sh)
        put(img, 7 + side * (spread + 2), 9, accent.mid)
    put(img, 7, 5, (74, 58, 66, 255))
    put(img, 8, 5, (74, 58, 66, 255))
    return img


PURPLE_WING = Ramp("#D3C2F5", "#B29AE8", "#8E72CC", "#6C53A3", "#4E3B78")
