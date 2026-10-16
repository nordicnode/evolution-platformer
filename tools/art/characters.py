"""The animated sprites: the player, the walker, the goal flag and the spawn pad.

Everything here is drawn by a small **puppet** — one function per creature, taking
"how far is it bobbing, where is each limb" — rather than one hand-typed 16x16 matrix
per frame. The reason is consistency, and it is the reason this module exists at all:
frames typed out separately drift, so frame four's head ends up a pixel lower than
frame three's and the character visibly shivers as it runs. A puppet cannot: every
frame of a cycle is the same body with different limb parameters.

Two conventions the renderer depends on:

* **Feet on row 15.** The game draws an actor at its physics box, and the box's
  bottom edge is the ground, so a sprite whose feet sit on the bottom row lands on the
  ground. Anything drawn below row 15 would sink into the terrain.
* **Facing right, always.** Every agent walks the level's own direction (fitness is
  progress to the right), so there is no mirrored art and no flip in the renderer.

The flag is split from the pole on purpose: the pole is drawn in two tiles
(`goal_top`, `goal_bottom`) because a goal is two tiles tall in the level format,
while the flag waves *inside* the upper tile, so its four frames are their own
sprites. That way a goal's flag can animate without the pole moving with it.
"""

from __future__ import annotations

from PIL import Image

from .common import (
    DIRT,
    DIRT_DARK,
    DIRT_MID,
    FLAG,
    FLAG_DARK,
    FLAG_LIGHT,
    GOLD,
    GOLD_DARK,
    PLAYER_BODY,
    PLAYER_DARK,
    PLAYER_EYE,
    PLAYER_LIGHT,
    PLAYER_OUTLINE,
    PLAYER_PUPIL,
    PLAYER_SHADE,
    PLAYER_TRIM,
    PLAYER_TRIM_DARK,
    POLE,
    POLE_DARK,
    POLE_LIGHT,
    POLE_SHADE,
    STONE_LIGHT,
    WALKER_BODY,
    WALKER_DARK,
    WALKER_EYE,
    WALKER_LIGHT,
    WALKER_PUPIL,
    WALKER_SHADE,
    cell,
)

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


def _bump(img: Image.Image, colour) -> Image.Image:
    """Wrap the finished figure in one pixel of `colour`, and return it.

    The outline is the reason the player reads on *every* backdrop rather than only on
    the ones its hue happens to suit: a body over grass and a body over sky are two
    different pictures, and a rim of near-black separates it from both. It is computed
    from the finished silhouette rather than drawn limb by limb, so every pose gets it
    — a trailing scarf, a raised knee, the crossed eyes of the hurt frame — and a change
    to the body moves the outline with it instead of leaving it behind.

    Pixels off the edge of the cell are simply not drawn: the feet stay on row 15 and
    the outline cannot sink into the ground under them. Only transparent pixels are
    written, so no detail inside the figure is ever covered up.
    """
    px = img.load()
    solid = [(x, y) for y in range(16) for x in range(16) if px[x, y][3] > 0]
    for x, y in solid:
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)):
            nx, ny = x + dx, y + dy
            if 0 <= nx < 16 and 0 <= ny < 16 and px[nx, ny][3] == 0:
                px[nx, ny] = colour
    return img


# --- the player -------------------------------------------------------------

#: The player's body, in the rows and columns the puppet draws it into. Top of the
#: head, the row the eyes sit on, the shoulders, the hip: everything else is derived,
#: so a change here moves every frame of every pose together.
HEAD_TOP = 2
HEAD_BOTTOM = 6
EYE_ROW = 4
HIP = 11
FOOT = 15


def _player(
    shift: int = 0,
    legs: tuple[tuple[int, int], tuple[int, int]] = ((5, 0), (8, 0)),
    arms: tuple[int, int] = (0, 0),
    eye: str = "open",
    head_tilt: int = 0,
) -> Image.Image:
    """One frame of the runner.

    `legs` is `((x, lift), (x, lift))` for the back leg and then the front one: how far
    left the leg hangs and how many pixels it is raised off the ground. `arms` is the
    vertical swing of each arm, negative being up. `shift` moves the whole upper body
    down, which is what gives a run cycle its weight — without it every frame looks
    like the same figure sliding sideways.
    """
    img = cell()
    px = img.load()
    hip = HIP + shift

    # Legs first, so the torso covers the tops of them. The back leg is a shade darker
    # than the front one: two legs of one colour read as one thick leg.
    for (x, lift), (body, boot) in zip(
        legs, ((PLAYER_DARK, PLAYER_SHADE), (PLAYER_BODY, PLAYER_DARK))
    ):
        bottom = FOOT - lift
        _box(px, x, hip - 1, x + 1, bottom, body)
        _box(px, x, bottom, x + 1, bottom, boot)
        _put(px, x + 1, bottom - 1, boot)

    # Torso, with the light on the side the sun is and the dark underneath it.
    _box(px, 4, 7 + shift, 9, 10 + shift, PLAYER_BODY)
    _box(px, 4, 7 + shift, 9, 7 + shift, PLAYER_LIGHT)
    _box(px, 4, 10 + shift, 9, 10 + shift, PLAYER_DARK)
    _box(px, 9, 7 + shift, 9, 10 + shift, PLAYER_DARK)

    # Arms: 1px each, swinging opposite the legs.
    back, front = arms
    for x, offset, colour in ((3, back, PLAYER_DARK), (10, front, PLAYER_BODY)):
        _box(px, x, 8 + shift + offset, x, 10 + shift + offset, colour)

    # The scarf: a band across the shoulders and a tail streaming behind the runner.
    _box(px, 3, 7 + shift, 10, 7 + shift, PLAYER_TRIM)
    _put(px, 10, 7 + shift, PLAYER_TRIM_DARK)
    scarf_wave = 1 if (legs[0][1] > 1 or shift > 0) else 0
    _put(px, 2, 7 + shift + scarf_wave, PLAYER_TRIM)
    _put(px, 2, 8 + shift + scarf_wave, PLAYER_TRIM_DARK)
    _put(px, 1, 8 + shift + scarf_wave, PLAYER_TRIM)
    _put(px, 1, 9 + shift, PLAYER_TRIM_DARK)

    # Head, with a rim of light along the top, headband, and the eye two pixels wide.
    top = HEAD_TOP + shift + head_tilt
    bottom = HEAD_BOTTOM + shift + head_tilt
    _box(px, 4, top, 9, bottom, PLAYER_BODY)
    _box(px, 4, top, 9, top, PLAYER_LIGHT)
    _box(px, 4, top + 1, 9, top + 1, PLAYER_TRIM)
    _put(px, 9, top + 1, PLAYER_TRIM_DARK)
    _box(px, 4, bottom, 9, bottom, PLAYER_DARK)
    _box(px, 9, top + 2, 9, bottom, PLAYER_DARK)
    _put(px, 5, top, PLAYER_LIGHT)

    eye_row = EYE_ROW + shift + head_tilt
    if eye == "open":
        _box(px, 6, eye_row, 8, eye_row + 1, PLAYER_EYE)
        _put(px, 7, eye_row, PLAYER_PUPIL)
        _put(px, 8, eye_row + 1, PLAYER_PUPIL)
    else:
        # A hurt eye is an X, which reads as "not running any more" at any zoom.
        for dx, dy in ((0, 0), (1, 1), (0, 1), (1, 0)):
            _put(px, 6 + dx, eye_row + dy, PLAYER_PUPIL)
    return _bump(img, PLAYER_OUTLINE)


def player_idle(sink: int) -> Image.Image:
    """Standing still: one pixel of breathing, nothing else."""
    return _player(shift=sink, arms=(1 if sink else 0, 1 if sink else 0))


def player_run(frame: int) -> Image.Image:
    """The six-frame run cycle.

    Contact, down, pass, contact, down, pass — the first three with the front leg
    leading and the last three with the back one, which is what makes it a *cycle*
    rather than two mirrored halves that never quite meet.
    """
    poses = (
        # (legs, arms, shift)
        (((4, 2), (9, 0)), (-1, 2), 0),
        (((5, 1), (8, 0)), (0, 1), 1),
        (((6, 0), (7, 2)), (1, 0), 0),
        (((8, 0), (5, 2)), (2, -1), 0),
        (((8, 0), (5, 1)), (1, 1), 1),
        (((7, 2), (6, 0)), (0, 1), 0),
    )
    legs, arms, shift = poses[frame % len(poses)]
    return _player(shift=shift, legs=legs, arms=arms)


def player_airborne(rising: bool) -> Image.Image:
    """Jumping (knees up, scarf trailing) or falling (legs reaching for the ground)."""
    if rising:
        return _player(legs=((5, 4), (8, 3)), arms=(-2, -2))
    return _player(legs=((4, 1), (10, 2)), arms=(1, -1))


def player_land() -> Image.Image:
    """The frame after touchdown: everything two pixels lower and wider."""
    return _player(shift=2, legs=((4, 0), (9, 0)), arms=(2, 2))


def player_hurt() -> Image.Image:
    """A death pose: slumped, arms up, eyes crossed out."""
    return _player(shift=1, legs=((5, 1), (8, 2)), arms=(-2, -2), eye="hurt")


# --- the walker -------------------------------------------------------------


def _walker(bob: int, feet: tuple[tuple[int, int], tuple[int, int]], dead: bool = False) -> Image.Image:
    """A patrolling blob, `bob` pixels lower, its two feet at the given squares."""
    img = cell()
    px = img.load()
    if dead:
        # Squashed flat: the corpse of a stomp should look like one, and a walker that
        # died is drawn under the living agents rather than beside them.
        _box(px, 2, 12, 13, 15, WALKER_BODY)
        _box(px, 2, 13, 13, 15, WALKER_DARK)
        _box(px, 3, 14, 12, 15, WALKER_SHADE)
        for x, y in ((4, 12), (5, 12), (10, 12), (11, 12)):
            _put(px, x, y, WALKER_DARK)
        return img

    top = 6 + bob
    bottom = 13 + bob
    # Dorsal horn-crests and rounded carapace.
    _put(px, 5, top - 2, WALKER_LIGHT)
    _put(px, 10, top - 2, WALKER_LIGHT)
    _box(px, 3, top, 12, bottom, WALKER_BODY)
    _box(px, 4, top - 1, 11, top - 1, WALKER_LIGHT)
    _box(px, 3, top, 12, top, WALKER_LIGHT)
    _box(px, 3, bottom, 12, bottom, WALKER_DARK)
    _box(px, 12, top, 12, bottom, WALKER_DARK)
    _box(px, 4, bottom - 1, 11, bottom - 1, WALKER_SHADE)

    # Expressive cyclops eye with brow and highlight.
    _box(px, 5, top + 1, 10, top + 1, WALKER_SHADE)
    _box(px, 6, top + 2, 9, top + 4, WALKER_EYE)
    _box(px, 7, top + 3, 8, top + 4, WALKER_PUPIL)
    _put(px, 6, top + 2, POLE_LIGHT)

    for x, y in feet:
        _box(px, x, y, x + 1, y, WALKER_DARK)
        _box(px, x, y - 1, x + 1, y - 1, WALKER_BODY)
    return _bump(img, PLAYER_OUTLINE)


# --- the goal ---------------------------------------------------------------


def _pole(px, colour: tuple[int, int, int, int], shade) -> None:
    """The flag's pole: two columns, lighter on the side the sun is."""
    _box(px, 4, 0, 4, 15, POLE_LIGHT)
    _box(px, 5, 0, 5, 15, colour)
    _box(px, 5, 0, 5, 15, shade)


def goal_top() -> Image.Image:
    """The top of the pole: a gold finial, no flag (the flag is its own sprite)."""
    img = cell()
    px = img.load()
    _box(px, 4, 3, 5, 15, POLE)
    _box(px, 4, 3, 4, 15, POLE_LIGHT)
    _box(px, 5, 3, 5, 15, POLE_DARK)
    _box(px, 3, 1, 6, 3, GOLD)
    _box(px, 3, 1, 6, 1, GOLD_DARK)
    _box(px, 4, 0, 5, 0, GOLD)
    return img


def goal_bottom() -> Image.Image:
    """The bottom of the pole, planted in a mound of earth."""
    img = cell()
    px = img.load()
    _box(px, 4, 0, 5, 11, POLE)
    _box(px, 4, 0, 4, 11, POLE_LIGHT)
    _box(px, 5, 0, 5, 11, POLE_DARK)
    _box(px, 4, 12, 5, 15, POLE_SHADE)

    _box(px, 2, 13, 7, 15, DIRT)
    _box(px, 3, 12, 6, 12, DIRT_MID)
    _box(px, 2, 15, 7, 15, DIRT_DARK)
    _box(px, 1, 14, 8, 15, DIRT_DARK)
    _put(px, 3, 13, DIRT_MID)
    _put(px, 6, 14, DIRT_MID)
    _put(px, 7, 12, STONE_LIGHT)
    return img


#: Where the flag's own rows sit: the wave moves inside these, so the pole and the
#: ground under it never move.
FLAG_TOP = 1
FLAG_BOTTOM = 7


def goal_flag(frame: int) -> Image.Image:
    """One frame of the waving flag, hung off the pole's right.

    Four frames is the cheapest wave that reads as a wave: the first and last are the
    extremes and the two in between are the travel, and the tip lags the body of the
    flag by one frame, which is what makes it look like cloth rather than a shape
    being scaled.
    """
    waves = (
        # (rows of the flag at the hoist, and at the tip)
        (2, 5, 3, 6),
        (1, 5, 2, 7),
        (1, 4, 3, 5),
        (2, 6, 2, 5),
    )
    hoist_top, hoist_bottom, tip_top, tip_bottom = waves[frame % len(waves)]
    img = cell()
    px = img.load()
    tip_x = 13
    for x in range(6, tip_x + 1):
        t = (x - 6) / (tip_x - 6)
        top = round(hoist_top + (tip_top - hoist_top) * t)
        bottom = round(hoist_bottom + (tip_bottom - hoist_bottom) * t)
        _box(px, x, top, x, bottom, FLAG)
        _put(px, x, top, FLAG_LIGHT)
        _put(px, x, bottom, FLAG_DARK)
        # The notch that stops a rectangle reading as a rectangle.
        if x > 10:
            _put(px, x, bottom - 1, FLAG_DARK if t > 0.6 else FLAG)
    # The hoist: the fold where the flag meets the pole.
    _box(px, 6, hoist_top, 6, hoist_bottom, FLAG_DARK)
    _put(px, 6, hoist_top + 1, FLAG_LIGHT)
    return img


def spawn_marker() -> Image.Image:
    """The editor's spawn pad: a gold landing pad with an arrow over it.

    Drawn only in the editor (the play and evolution views do not show it), so it is
    allowed to be louder than anything that ships on screen during a run.
    """
    img = cell()
    px = img.load()
    _box(px, 1, 12, 14, 15, GOLD_DARK)
    _box(px, 1, 12, 14, 12, GOLD)
    _box(px, 1, 15, 14, 15, GOLD_DARK)
    for x in range(2, 14, 3):
        _box(px, x, 13, x + 1, 14, GOLD)
    # An arrow, so the pad says which way the run goes.
    for step in range(4):
        _put(px, 7 - step, 8 + step, GOLD)
        _put(px, 8 + step, 8 + step, GOLD)
    _box(px, 7, 8, 8, 8, GOLD)
    return img


def sprites() -> list[tuple[str, Image.Image]]:
    """Every character sprite, in the order the manifest lists them."""
    out: list[tuple[str, Image.Image]] = [
        ("player_idle_a", player_idle(0)),
        ("player_idle_b", player_idle(1)),
    ]
    for frame in range(6):
        out.append((f"player_run_{frame + 1}", player_run(frame)))
    out += [
        ("player_jump", player_airborne(rising=True)),
        ("player_fall", player_airborne(rising=False)),
        ("player_land", player_land()),
        ("player_hurt", player_hurt()),
        ("walker_a", _walker(0, ((3, 15), (8, 15)))),
        ("walker_b", _walker(1, ((4, 15), (9, 15)))),
        ("walker_c", _walker(0, ((3, 14), (9, 15)))),
        ("walker_d", _walker(1, ((5, 15), (10, 15)))),
        ("walker_dead", _walker(0, (), dead=True)),
        ("goal_top", goal_top()),
        ("goal_bottom", goal_bottom()),
    ]
    for frame in range(4):
        out.append((f"goal_flag_{frame + 1}", goal_flag(frame)))
    out.append(("spawn_marker", spawn_marker()))
    return out
