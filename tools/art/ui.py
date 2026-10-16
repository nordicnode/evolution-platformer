"""The window's own tool glyphs: the icons the app's controls are drawn with.

Everything else in the atlas belongs to the *world* — terrain, characters, props, the dust a
jump kicks up. These nine are the only sprites in the game that the game never draws. They are
the pictures on the application's own controls, and they live in the atlas rather than in the app
for one reason: the window's chrome is a row of plates carrying sprites out of the *shipped*
atlas (`crate::ui::icon_button`), which is what makes a control read as a piece of the
same game as the level beside it rather than as a glyph in whatever font the UI toolkit happens
to ship. A button whose icon came from somewhere else would be the one picture in the window
drawn in a different hand.

The family splits in two. The first five are the *document's* tools — a turn arrow each way, a
frame, a magnifier, a restart. The last four are the *run's*: pause and play, a single step, and
a medal for the champion a run can write out. Nothing draws the world with any of them; the
editor's bar and the evolution panel are two rows of the same kind of object, and giving the
second row pictures as well is what keeps a run's menu from reading as a form.

**One tone, deliberately.** Each glyph is a single flat ink — `PLAYER_OUTLINE`, the palette's
near-black, which is also the colour of the window's own type — on a transparent cell, and the
*plate* behind it carries every bit of the state: resting, hovered, pressed, selected and
disabled are the app's business, and it draws the same picture in all five. The two things that
make a second tone tempting are both reasons not to have one. A highlight along a glyph's top
edge is invisible on a cream plate (the two colours are four points apart) and nearly invisible
on the gold one, so it would be a pixel of noise in one state and a ghost in another; and an
icon that drew itself paler on a disabled plate would be a control claiming an attention it
cannot honour, which the app does instead by tinting the whole glyph's alpha — `Color32` cannot
lighten a dark picture by multiplying it, so the exported sprite has to stay the darkest thing
it can be.

**Clearance.** Every glyph is drawn inside the same one-pixel margin as every other sprite, and
none of them is a filled square: a tool icon that ran to the edge of its cell would touch the
lip of the plate it sits in, which is the one thing that makes a row of buttons look printed
rather than assembled.

**Why they sit just before the seams.** `tools/gen_tileset.py` puts the material seams last
because they are the only sprites drawn *over* another family's art; every family added after
them would be a family whose row sits between the seams and the sprites they blend. So a new
family that is not a seam goes in front of them, and this one does.

The drawing itself is geometric — a ring, an arc swept clockwise between two angles, a triangle
— rather than hand-typed 16-row string art, because an arrow is a shape with a *relationship*
in it: the two halves of the turn arrow have to be exact mirrors of each other, and a rounded
arc typed out one row at a time is where a pixel of difference between undo and redo comes
from.
"""

from __future__ import annotations

from math import atan2, cos, degrees, radians, sin

from PIL import Image

from .common import PLAYER_OUTLINE, cell

#: The one colour every glyph in this family is drawn in. Named here rather than at each call
#: site so the "one tone" rule above is a single fact about the module.
INK = PLAYER_OUTLINE


# --- a drawing surface ------------------------------------------------------


def _put(px, x: int, y: int, colour) -> None:
    """One pixel, ignored when it falls outside the cell."""
    if 0 <= x < 16 and 0 <= y < 16:
        px[x, y] = colour


def _box(px, x0: int, y0: int, x1: int, y1: int, colour) -> None:
    """A filled rectangle, inclusive of both corners."""
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            _put(px, x, y, colour)


def _ring(px, cx: float, cy: float, outer: float, inner: float, colour) -> None:
    """A thick circle: the pixels whose distance from the centre lies between the two radii."""
    for y in range(16):
        for x in range(16):
            dx, dy = (x + 0.5) - cx, (y + 0.5) - cy
            if inner <= (dx * dx + dy * dy) ** 0.5 <= outer:
                _put(px, x, y, colour)


def _arc(px, cx: float, cy: float, outer: float, inner: float, a0: float, a1: float, colour) -> None:
    """The part of a ring swept **clockwise** from `a0` to `a1`, in degrees.

    Angles are screen angles: zero points east and grows southwards, so a growing angle reads
    as clockwise on the screen. Clockwise and not "the short way round", because which way an
    arrow goes round its circle is the whole of what the arrow means: `180 -> 55` has to be the
    arc over the *top*, and the short way from 180 to 55 is under the bottom.
    """
    sweep = (a1 - a0) % 360.0
    for y in range(16):
        for x in range(16):
            dx, dy = (x + 0.5) - cx, (y + 0.5) - cy
            if not inner <= (dx * dx + dy * dy) ** 0.5 <= outer:
                continue
            angle = degrees(atan2(dy, dx)) % 360.0
            if (angle - a0) % 360.0 <= sweep:
                _put(px, x, y, colour)


def _triangle(px, a: tuple[float, float], b: tuple[float, float], c: tuple[float, float], colour) -> None:
    """A filled triangle, each pixel tested at its own centre."""
    for y in range(16):
        for x in range(16):
            p = (x + 0.5, y + 0.5)

            def side(u, v) -> float:
                return (p[0] - v[0]) * (u[1] - v[1]) - (u[0] - v[0]) * (p[1] - v[1])

            edges = (side(a, b), side(b, c), side(c, a))
            if not (any(e < 0 for e in edges) and any(e > 0 for e in edges)):
                _put(px, x, y, colour)


def _head(px, tip: tuple[float, float], travel: float, width: float, length: float, colour) -> None:
    """An arrowhead: a triangle whose point is `tip`, its tail `length` behind it along `travel`
    degrees, and its two barbs `width` apart.

    Taken as a tip and a direction rather than as three points so that an arrow's head is placed
    by *where it is going*: the head of an arc is put at the end of the arc's centre line and
    aimed the way the arc is travelling, which is the relationship that has to hold for the
    arrow to look like it is going somewhere rather than like a triangle sitting nearby.
    """
    ux, uy = cos(radians(travel)), sin(radians(travel))
    tail = (tip[0] - ux * length, tip[1] - uy * length)
    # The barbs go along the perpendicular of the travel direction, half the width each side.
    nx, ny = -uy, ux
    half = width * 0.5
    _triangle(
        px,
        tip,
        (tail[0] + nx * half, tail[1] + ny * half),
        (tail[0] - nx * half, tail[1] - ny * half),
        colour,
    )


# --- the document tools -----------------------------------------------------

#: The turn arrow's geometry, shared by the two directions of it. A ring of radius 5 with a
#: 2-pixel band, an arc over the top, and a head 7 points from the centre — which is the
#: furthest the point of an arrow can reach into a 16-pixel cell and still leave the outermost
#: column of pixels clear, so it is this shape's own constraint rather than a taste.
TURN_RADIUS_OUTER = 5.0
TURN_RADIUS_INNER = 3.0
TURN_REACH = 7.0
TURN_HEAD_WIDTH = 4.2
TURN_HEAD_LENGTH = 3.0


def _turn_arrow(mirror: bool) -> Image.Image:
    """One of the two turn arrows. `mirror` reflects the shape about the vertical axis.

    Reflected as *angles and a tip* (`a` becomes `180 - a`, the tip moves to the other side of
    the centre) rather than by flipping the finished image, so the two sprites are exact mirrors
    of each other by construction and stay that way if either radius is ever retuned. Note what
    a reflection does to a sweep: it runs the other way round, so the two ends swap as well —
    the mirrored arc is the one from `180 - a1` to `180 - a0`, not from `180 - a0` to
    `180 - a1`, and getting that backwards draws the arrow round the *bottom* of its circle,
    which is the difference between "step back" and "step forward".
    """
    img = cell()
    px = img.load()
    cx, cy = 8.0, 8.0

    def across(angle: float) -> float:
        return (180.0 - angle) % 360.0

    # West, over the top, down to the lower right, with the head at the western end: the head
    # goes where the arrow is *going*, and this one is going back.
    a0, a1 = (across(55.0), across(180.0)) if mirror else (180.0, 55.0)
    _arc(px, cx, cy, TURN_RADIUS_OUTER, TURN_RADIUS_INNER, a0, a1, INK)
    tip = (cx + (-TURN_REACH if not mirror else TURN_REACH), cy)
    _head(px, tip, across(180.0) if mirror else 180.0, TURN_HEAD_WIDTH, TURN_HEAD_LENGTH, INK)
    return img


def undo() -> Image.Image:
    """A counter-clockwise arrow: an arc over the top with its head at the left-hand end."""
    return _turn_arrow(mirror=False)


def redo() -> Image.Image:
    """The same arrow the other way round, for the control that steps forward again."""
    return _turn_arrow(mirror=True)


def frame() -> Image.Image:
    """Four corner brackets around a block in the middle: "make the whole level fit the view".

    The only glyph in the family that is not a circle, and the one that has to be symmetrical
    both ways: it is read as a frame, and a frame with one arm a pixel longer than its opposite
    is a frame that looks broken. Every rectangle below is mirrored about the cell's own centre
    line (column `x` pairs with column `15 - x`), which is the arithmetic a reader's eye does
    instantly and cannot do by accident.
    """
    img = cell()
    px = img.load()
    # Two pixels thick, three long, one pair to a corner: the horizontal and vertical arms of
    # each bracket, and the block of level they are framing.
    _box(px, 2, 2, 4, 3, INK)
    _box(px, 2, 2, 3, 4, INK)
    _box(px, 11, 2, 13, 3, INK)
    _box(px, 12, 2, 13, 4, INK)
    _box(px, 2, 12, 4, 13, INK)
    _box(px, 2, 11, 3, 13, INK)
    _box(px, 11, 12, 13, 13, INK)
    _box(px, 12, 11, 13, 13, INK)
    _box(px, 6, 6, 9, 9, INK)
    return img


def magnifier() -> Image.Image:
    """A magnifying glass: a ring with a handle, for "show me what this level is made of".

    Drawn off-centre on purpose and the only glyph here that is. A magnifier is a ring with a
    *stick*, so the stick has to have somewhere to go: the lens is up and to the left and the
    handle runs into the bottom-right corner, which is how the shape is read at any size. The
    handle starts on the lens rather than beside it — one pixel of sky between a glass and its
    handle turns the pair into a circle and a line.
    """
    img = cell()
    px = img.load()
    _ring(px, 7.0, 7.0, 4.6, 2.8, INK)
    # The handle: overlapping two-by-two blocks down the diagonal, which is a 2-pixel-wide bar
    # without needing a rotated rectangle. It reaches the cell's second-to-last column, so the
    # outer ring of the cell stays clear like every other sprite's margin.
    for step in range(4):
        _box(px, 10 + step, 10 + step, 11 + step, 11 + step, INK)
    return img


def restart() -> Image.Image:
    """A circular arrow: the nearly-closed ring an eye reads as "again".

    Open at the top and nowhere else, because a ring with a gap anywhere else is a letter. The
    head sits in the gap and points along the way round, which is the one thing that separates
    this from the turn arrows: they are a *hook* with a head at its end, this is a circle that
    has lost a bite of itself.
    """
    img = cell()
    px = img.load()
    cx, cy = 8.0, 8.0
    # From the upper right, the long way round, to the upper left: a 290-degree ring whose gap
    # is the 70 degrees the head goes in.
    _arc(px, cx, cy, 5.2, 3.2, 315.0, 245.0, INK)
    # The head is aimed along the circle's own tangent (245 + 90), so it points the way the
    # arrow travels rather than outwards, which is what an arrow on a circle means.
    _head(px, (9.3, 2.8), 335.0, 4.2, 3.4, INK)
    return img


def pause() -> Image.Image:
    """Two upright bars: the transport control every machine has agreed on since tape."""
    img = cell()
    px = img.load()
    _box(px, 4, 3, 6, 12, INK)
    _box(px, 9, 3, 11, 12, INK)
    return img


def play() -> Image.Image:
    """A right-pointing triangle, and the same one the step glyph is built from.

    Deliberately larger than the pause bars it alternates with: the two are *one* control that
    flips its picture, and a triangle inscribed in the same box as two bars reads as smaller than
    them — the eye measures the ink, not the box.
    """
    img = cell()
    px = img.load()
    _triangle(px, (12.5, 8.0), (4.0, 2.5), (4.0, 13.5), INK)
    return img


def step() -> Image.Image:
    """A triangle against a bar: the transport symbol for "one of them, then stop".

    The bar on the right is what makes it a *step* rather than a play — take the play triangle
    away and it is a pause, put it on both sides and it is a skip-to-end. A run watched one
    generation at a time is the whole reason this exists.
    """
    img = cell()
    px = img.load()
    _triangle(px, (10.5, 8.0), (3.0, 3.0), (3.0, 13.0), INK)
    _box(px, 12, 3, 13, 12, INK)
    return img


def medal() -> Image.Image:
    """A rosette on two ribbons: the champion a run can write out to disk.

    Not a save icon, because the window already has one of those and it means the *document* —
    the crate the tool bar saves the level with. This one is a run's best agent leaving the
    building, so it is the one shape in the family that is unmistakably a prize: a badge with a
    centre to it, hung off two strips of ribbon.
    """
    img = cell()
    px = img.load()
    # The ribbons first, so the badge sits over their upper ends rather than under them.
    _box(px, 4, 1, 5, 5, INK)
    _box(px, 10, 1, 11, 5, INK)
    _ring(px, 8.0, 9.0, 4.8, 2.8, INK)
    _box(px, 7, 8, 8, 10, INK)
    return img


def mirror() -> Image.Image:
    """Two halves leaving a dashed axis: the shape every drawing tool means by "flip it sideways".

    A reflection icon is not a picture of a reflection — it is a *pair*: two halves leaving one
    line in opposite directions. Three decisions make the pair read as one idea at sixteen
    pixels. The axis is **dashed** rather than solid, because a bar between two triangles is a
    shape with a seam down the middle and at this size the seam is all an eye sees. The
    triangles are **stubby** (five columns wide, ten tall) so the pair fills the cell without
    their tips colliding with the axis. And the whole glyph is the one place in this family where
    the two halves are *not* reflections of each other in angle: they are reflections of each
    other in *position only*, each pointing away from the line it is leaving — which is what
    says "these two are the same shape, one to each side", and is the thing a single arrow
    cannot say at any size.
    """
    img = cell()
    px = img.load()
    # The axis: three dashes down the cell's own centre column, so the two halves are separated
    # by the same amount at every height they are read at.
    for y0 in (2, 7, 12):
        _box(px, 8, y0, 8, y0 + 1, INK)
    # Two triangles tip-to-tip across the dashes, each mirrored about the cell's own centre
    # column: the left one's tip is at 6.5 and its base at 2.0, so the right one's are at 9.5 and
    # 14.0 — equal and opposite, which is what makes the pair read as *one* shape. Getting the
    # base a pixel short (13.0) draws a right triangle a column narrower than its left twin, and
    # at sixteen pixels that is the whole glyph looking bent.
    _triangle(px, (6.5, 8.0), (2.0, 3.0), (2.0, 13.0), INK)
    _triangle(px, (9.5, 8.0), (14.0, 3.0), (14.0, 13.0), INK)
    return img


def sprites() -> list[tuple[str, Image.Image]]:
    """Every glyph, in the order the manifest lists them.

    The `ui_` prefix is not decoration: the manifest is one flat namespace shared with the
    world's sprites, and the app asks for these ten by name while the renderer asks for the
    other two hundred — a naming that says which half of the atlas a name belongs to is what
    keeps "the crate the toolbar uses" from ever meaning "the crate the level places".
    """
    return [
        ("ui_mirror", mirror()),
        ("ui_undo", undo()),
        ("ui_redo", redo()),
        ("ui_fit", frame()),
        ("ui_info", magnifier()),
        ("ui_restart", restart()),
        ("ui_pause", pause()),
        ("ui_play", play()),
        ("ui_step", step()),
        ("ui_medal", medal()),
    ]
