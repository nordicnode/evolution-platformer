"""The shared vocabulary every tileset module draws with.

Three things live here, and nothing else may duplicate them:

* **The colours.** Every sprite in the game is drawn from this dictionary, so the
  terrain, the characters and the effects cannot drift into three different
  palettes. A module maps its own single-character keys onto these names (see the
  `PALETTE` at the top of each art module), which is also why a module can reuse
  the letter `b` for something else without clashing with anybody.
* **`art(rows, palette)`**, which turns 16 rows of 16 keys into a 16x16 cell and
  refuses anything the wrong shape. A sprite matrix is a picture; the failure mode
  of a picture typed by hand is a row that is 15 characters long, and that is a
  `ValueError` here rather than a silently crooked sprite in the atlas.
* **`h(x, y, seed)`**, deterministic value noise in [0, 1). The speckle in a dirt
  tile and the per-tile variant choice come from it, so regenerating the atlas can
  never produce a diff (the tool is checked in, and a tool that churns is a tool
  nobody can review).

The atlas writes what these modules return; `tools/gen_tileset.py` owns the grid
and the manifest, and `assets/README.md` lists what each name is for.
"""

from __future__ import annotations

from PIL import Image

CELL = 16

TRANSPARENT = (0, 0, 0, 0)

# --- sky and backdrop ------------------------------------------------------
SKY = (94, 168, 231)
SKY_TOP = (78, 152, 226)
SKY_BOTTOM = (186, 222, 244)
CHECKER = (196, 220, 238)
HILL_FAR = (128, 178, 224)
HILL_NEAR = (96, 152, 208)
SUN = (255, 240, 190)

# --- terrain: grass over dirt, over stone --------------------------------
GRASS_LIGHT = (150, 220, 96)
GRASS = (104, 186, 68)
GRASS_MID = (74, 152, 54)
GRASS_DARK = (52, 112, 44)
GRASS_EDGE = (36, 84, 36)

DIRT_LIGHT = (188, 140, 92)
DIRT = (152, 106, 66)
DIRT_MID = (126, 84, 52)
DIRT_DARK = (98, 62, 40)
DIRT_DEEP = (72, 44, 30)

STONE_LIGHT = (176, 180, 190)
STONE = (140, 145, 158)
STONE_MID = (112, 118, 132)
STONE_DARK = (84, 90, 104)
STONE_DEEP = (60, 66, 78)

# --- terrain: sand over packed sand, over sandstone ------------------------
# A third material, and the reason it is a *material* rather than a tint of the second: a
# beach is not dirt in yellow, it is a different picture — sand piled by wind with no grass
# on it, and the crown is the part an eye reads as "beach" from three screens away.
#
# The tones are the dirt family's warmth pushed towards white and down in saturation, which
# is what sun does to stone: bright at the top where the light lands, visibly packed a few
# rows down, and a warm pale rock underneath rather than the grey one under a hill.
#
# It deliberately stays clear of the player. Vermilion on sand is a warm body on a warm
# ground, and the runner is the one sprite that must never blend into what it runs on, so
# the sand is pale and low-saturation where the runner is dark and saturated — a distance
# the material test (`view::tests`) holds rather than an eye.
SAND_LIGHT = (252, 235, 184)
SAND = (241, 217, 152)
SAND_MID = (224, 196, 128)
SAND_DARK = (196, 166, 104)
SAND_DEEP = (162, 134, 84)

SANDSTONE_LIGHT = (228, 206, 162)
SANDSTONE = (202, 178, 132)
SANDSTONE_MID = (176, 152, 110)
SANDSTONE_DARK = (144, 122, 86)
SANDSTONE_DEEP = (114, 94, 66)

# A shell and a few grits on the beach: the sand family's own variant detail, in the cream
# and rust of the rest of the palette rather than a new colour family.
SHELL = (246, 226, 206)
SHELL_DARK = (206, 176, 148)

# --- terrain: brick & masonry, and wood planks ---------------------------
BRICK_TOP = (214, 118, 92)
BRICK_LIGHT = (196, 98, 74)
BRICK = (168, 76, 56)
BRICK_MID = (138, 58, 44)
BRICK_DARK = (106, 42, 34)
BRICK_DEEP = (76, 28, 24)
MORTAR = (196, 186, 172)
MORTAR_DARK = (148, 138, 126)

MASONRY_LIGHT = (156, 152, 164)
MASONRY = (126, 122, 136)
MASONRY_MID = (98, 94, 108)
MASONRY_DARK = (74, 70, 84)
MASONRY_DEEP = (52, 48, 60)

WOOD_TOP = (218, 168, 108)
WOOD_LIGHT = (194, 142, 86)
WOOD = (164, 114, 66)
WOOD_MID = (134, 90, 50)
WOOD_DARK = (102, 66, 36)
WOOD_DEEP = (72, 44, 24)
IRON_BOLT = (92, 98, 112)
IRON_LIGHT = (146, 154, 168)

# --- new prop accents: lanterns, crystals, banners, crates ---------------
CRYSTAL_LIGHT = (176, 244, 255)
CRYSTAL = (96, 212, 238)
CRYSTAL_DARK = (48, 148, 184)
LANTERN_GLOW = (255, 236, 148)
LANTERN_WARM = (255, 178, 72)
IRON_FRAME = (58, 62, 74)

# --- the player: a vermilion runner with a cream scarf -------------------
# Chosen against the level rather than in the abstract. The backdrop is a light blue sky
# over green hills, and the ground is green over brown, so a *cool* character — which is
# what this used to be, a cyan body a few shades off the sky it runs in front of — is the
# one thing that disappears. Vermilion is close to the complement of all of those, and it
# is the most saturated hue in the palette, which is what a reader needs from the one
# sprite that is always moving.
#
# It stays clear of the other characters: the walker is purple, the flag red, the goal's
# finial and the spawn pad gold, the spikes stone. Cream rather than gold for the scarf
# keeps the gold family meaning "the goal", and the outline below — not the hue — is what
# carries the silhouette wherever the body sits.
PLAYER_BODY = (244, 128, 52)
PLAYER_LIGHT = (255, 186, 116)
PLAYER_DARK = (186, 74, 30)
PLAYER_SHADE = (120, 42, 22)
#: One pixel of near-black around the whole figure (`characters._bump`). Warm-dark rather
#: than pure black so it sits in the same palette as the rest of the art.
PLAYER_OUTLINE = (44, 24, 36)
PLAYER_EYE = (255, 252, 246)
PLAYER_PUPIL = (32, 22, 36)
PLAYER_TRIM = (255, 246, 222)
PLAYER_TRIM_DARK = (216, 176, 132)

# --- the walker: a purple patroller --------------------------------------
WALKER_BODY = (188, 104, 220)
WALKER_LIGHT = (222, 168, 244)
WALKER_DARK = (124, 58, 154)
WALKER_SHADE = (88, 36, 116)
WALKER_EYE = (255, 246, 126)
WALKER_PUPIL = (48, 24, 64)

# --- the goal flag -------------------------------------------------------
FLAG = (232, 80, 88)
FLAG_LIGHT = (252, 138, 130)
FLAG_DARK = (168, 44, 52)
POLE = (236, 230, 214)
POLE_LIGHT = (255, 252, 244)
POLE_DARK = (178, 170, 152)
POLE_SHADE = (128, 122, 108)
GOLD = (255, 214, 102)
GOLD_DARK = (206, 156, 44)

# --- the far tree-line ---------------------------------------------------
# A conifer three hills away is a silhouette, so these are cool and close to each other:
# they are read against pale blue hills and a pale sky, and the *shape* is what carries
# them. Green enough to say "forest", blue enough to sit behind the hills rather than in
# front of them.
#
# They are also **hazy**, which they were not at first: a distance makes things paler and
# bluer, and a tree-line drawn in dark saturated green against hills that have faded reads
# as being in front of them — the whole depth cue the parallax is paying for, spent on the
# wrong side.
#
# The body is the conifer green mixed 62% of the way to the far hills' own colour as the
# renderer draws it (`view::HILL_FAR`, 126, 168, 186), and the other three tones spread
# around it by fixed steps rather than being mixed by the same fraction — a haze that shrinks
# the *shading* as much as the hue leaves a flat blob where a conifer's tiers should be. So
# the tones are 40, 49 and 61 away from the hills where the old ones were 69, 102, 130 and
# 161: the trunk is still the darkest thing in the tree-line, because it is the part that has
# to read against the hill it is standing on, and nothing here is darker than that.
#
# Lightened again in the parallax pass, and greened with it: the previous tones
# (117/106/99/92 in the red channel, blue nearly as high as green) read on screen as a
# dark blue-grey mass rather than as trees — the tiers alternate the body and the
# shadow, so half of every crown was the darkest tone, and a crown that is half
# shadow at a luminance near 120 is a silhouette with the light off. Every tone is
# roughly twenty points brighter now and green is the strongest channel in all of
# them, which is what a conifer in haze actually looks like against a blue hill:
# paler than the hill is blue, and greener than it.
PINE_LIGHT = (142, 180, 168)
PINE = (128, 172, 158)
PINE_DARK = (116, 160, 148)
PINE_SHADE = (104, 148, 138)

# The rank behind that one, a further step into the haze: two tones rather than four, since
# four tones of shading is detail an eye that far away does not get, and the rank behind is
# the one the haze eats first. Closer to the hills again — 20 and 31 away from them, where
# the near rank is 39 and 64 — because the whole point of a second rank is that it is the
# *farther* of the two, and tone is how a flat picture says so.
PINE_FAR_LIGHT = (140, 180, 182)
PINE_FAR = (130, 170, 174)

# The rank on the *near* hills: the same conifer, one band closer, so the haze is
# mostly off it — but only mostly, and the "mostly" matters. The first version of
# this palette was the conifer at full saturation (84, 158, 88 with a lime light
# tone), and at that strength a near tree stops being backdrop: it is the most
# saturated thing in the scene, it stands in front of a hazed hill in an unhazed
# green, and the eye reads it as a foreground object that happens to be standing
# on the horizon. So these tones are that same green mixed half way to the near
# hills' own colour as the renderer draws it (`view::HILL_NEAR`, 96, 146, 132) —
# the far rank's own method, a shorter walk. Green stays the strongest channel
# and the rank stays darker and more saturated than the far rank tone for tone
# (that ordering is the depth cue), but the blue channel comes up by twenty-odd
# points, the lime goes out of the light tone, and the spread between the light
# and dark tones halves: less internal contrast is flatter, and flatter is
# farther.
PINE_NEAR_LIGHT = (100, 161, 118)
PINE_NEAR = (90, 152, 110)
PINE_NEAR_DARK = (82, 142, 103)

# The near rank's trunk: the foreground trees' `BARK` (118, 82, 54) faded the
# same way in spirit but not in direction. Mixing bark towards the hills would
# turn it green — the exact complaint the brown trunk replaced — because the
# hills are green; so it is faded towards its own grey instead: same warm
# ordering (red over green over blue), about half the saturation, which is a
# bark seen through air rather than a bark in the hand.
PINE_NEAR_TRUNK = (108, 88, 72)

# --- props, decor and effects -------------------------------------------
LEAF_LIGHT = (132, 204, 96)
LEAF = (84, 164, 72)
LEAF_DARK = (52, 116, 56)
PETAL = (255, 226, 138)
PETAL_PINK = (250, 168, 196)
PETAL_DARK = (206, 130, 60)
ROCK_LIGHT = (168, 172, 182)
ROCK = (132, 137, 150)
ROCK_DARK = (96, 101, 114)
MUSHROOM_CAP = (226, 96, 96)
MUSHROOM_LIGHT = (248, 170, 150)
MUSHROOM_STEM = (238, 226, 200)
CLOUD_LIGHT = (255, 255, 255)
CLOUD = (226, 238, 250)
CLOUD_DARK = (186, 208, 232)
DUST_LIGHT = (236, 224, 202)
DUST = (204, 188, 162)
DUST_DARK = (168, 150, 126)
SPARK = (255, 248, 214)
SPARK_WARM = (255, 214, 120)
TUFT = (110, 186, 74)
TUFT_DARK = (74, 144, 54)
# Long dry blades: the tuft family's third shape leans towards straw rather than towards
# another shade of green, so a lawn of grass sprites has something in it that is not one
# plant repeated. It is also the tone the reeds already use, kept here so a field of grass
# and a patch of reeds are recognisably the same world.
TUFT_DRY = (166, 178, 92)
TUFT_DRY_DARK = (120, 134, 62)

# --- the foreground wood --------------------------------------------------
# The trees of a level are the *near* green, and the bushes are already drawn in it: a tree
# whose foliage was a different green from the bush at its foot would read as two plants
# that happen to share a trunk. What a tree adds is bark and a deep underside, because a
# canopy four tiles wide is the first thing in the game with a shadow under it.
BARK_LIGHT = (150, 110, 74)
BARK = (118, 82, 54)
BARK_DARK = (86, 58, 38)
LEAF_DEEP = (38, 88, 48)

# --- butterflies ----------------------------------------------------------
# Ambient life, and the one sprite in the game that is *supposed* to be small and moving, so
# it is drawn as a bright pair of wings and almost no body. Pale gold for the common one,
# pale violet for the other: both are far from vermilion and both are far from the sky, which
# are the two things a creature drifting over an open level is always seen against.
BUTTERFLY_WING = (252, 238, 186)
BUTTERFLY_WING_DARK = (224, 192, 106)
BUTTERFLY_EDGE = (152, 120, 64)
BUTTERFLY_PALE = (232, 220, 252)
BUTTERFLY_PALE_DARK = (178, 160, 226)
BUTTERFLY_BODY = (74, 58, 46)


def cell() -> Image.Image:
    """A blank 16x16 cell."""
    return Image.new("RGBA", (CELL, CELL), TRANSPARENT)


def art(rows: list[str], palette: dict[str, tuple[int, int, int, int]]) -> Image.Image:
    """Draw a sprite from 16 rows of 16 palette keys.

    Every key must be in `palette`; every row must be `CELL` characters. Both are
    hard errors, because a sprite that is one character short is a sprite whose art
    is wrong in a way that is very hard to see in a 16-pixel cell.
    """
    if len(rows) != CELL:
        raise ValueError(f"expected {CELL} rows, got {len(rows)}")
    img = cell()
    px = img.load()
    for y, row in enumerate(rows):
        if len(row) != CELL:
            raise ValueError(f"row {y} is {len(row)} wide, expected {CELL}: {row!r}")
        for x, key in enumerate(row):
            if key not in palette:
                raise KeyError(f"row {y} column {x}: {key!r} is not in the palette")
            px[x, y] = palette[key]
    return img


def h(x: int, y: int, seed: int) -> float:
    """Deterministic value noise in [0, 1) from integer coordinates."""
    n = (x * 73856093) ^ (y * 19349663) ^ (seed * 83492791)
    n &= 0xFFFFFFFF
    n ^= n >> 13
    n = (n * 1274126177) & 0xFFFFFFFF
    n ^= n >> 16
    return (n & 0xFFFF) / 65536.0


def rgba(colour: tuple[int, int, int]) -> tuple[int, int, int, int]:
    """An RGB constant as opaque RGBA, for a palette table."""
    return (colour[0], colour[1], colour[2], 255)
