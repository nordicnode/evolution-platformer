"""Scenery, effects and the window's glyphs.

Props are the layer that tells a player the world existed before they arrived, so
each one is drawn with the same three-tone lighting as the terrain: a leaf mass is a
union of lobes with a lit upper-left rim and a shadowed underside, a stone is the
material's own ramp, and nothing is wrapped in ink. The shapes are deliberately
smaller than their cells — `view::prop_size` draws them at their own scale — so they
are drawn bottom-aligned and allowed to breathe at the top.
"""

from __future__ import annotations

import math

from PIL import Image, ImageDraw

from .kit import (
    CLEAR,
    CORAL,
    CREAM,
    CYAN,
    GOLD,
    GRASS,
    IRON,
    LEAF,
    LEAF_DARK,
    SAND,
    SOIL,
    STONE,
    TEAL,
    WHITE,
    WOOD,
    canvas,
    chance,
    put,
    rect,
    rgba,
    tint,
)

BARK = type(CORAL)("#9C7350", "#825C40", "#684832", "#4E3628", "#38261D")
STRAW = type(CORAL)("#E8D79A", "#D2BC79", "#B39A5D", "#8E7848", "#6B5936")
PINK = type(CORAL)("#FFD3E4", "#F2A8C4", "#D97FA6", "#B25F84", "#8A4565")
STONE_PROP = STONE


def _union(boxes, shape: str = "ellipse") -> Image.Image:
    mask = Image.new("L", (16, 16), 0)
    draw = ImageDraw.Draw(mask)
    for box in boxes:
        if shape == "ellipse":
            draw.ellipse(box, fill=255)
        else:
            draw.rectangle(box, fill=255)
    return mask


def _shade_union(mask: Image.Image, ramp, lit: str = "tl", speckle: bool = True) -> Image.Image:
    """Give a silhouette volume from its own shape: no outline, one light.

    The top and left rims take the lit tone, the bottom takes the shadow tone, and the
    body is the mid. Speckles break up flat areas so a bush reads as leaves rather than
    as a green oval.
    """
    img = canvas()
    px = mask.load()

    def inside(x: int, y: int) -> bool:
        return 0 <= x < 16 and 0 <= y < 16 and px[x, y] > 0

    for y in range(16):
        for x in range(16):
            if not inside(x, y):
                continue
            above, left = inside(x, y - 1), inside(x - 1, y)
            below, right = inside(x, y + 1), inside(x + 1, y)
            if not above:
                tone = ramp.hi
            elif not left and lit == "tl":
                tone = ramp.lit
            elif not below or not right:
                tone = ramp.sh
            else:
                tone = ramp.mid
            if speckle and tone == ramp.mid:
                # Leaf texture: a hashed handful of shadow pixels, not a lattice of
                # polka dots. The first pass used `(x * 7 + y * 13) % 11`, which is a
                # regular grid and read as a dotted pattern on every bush.
                if (x * 37 + y * 91) % 23 < 2:
                    tone = ramp.sh
                elif (x * 53 + y * 17) % 41 == 0:
                    tone = ramp.hi
            put(img, x, y, tone)
    return img


def _leaf_mass(boxes, ramp=LEAF) -> Image.Image:
    return _shade_union(_union(boxes), ramp)


def bush(shape: int, frame: int) -> Image.Image:
    """Six bushes: plain, berry, and wide; each in two frames of sway."""
    sway = 1 if frame else 0
    kind = shape // 2
    img = canvas()
    if kind == 0:  # plain
        boxes = [(1, 7, 8, 15), (6, 4, 14, 15), (3, 5, 12, 14)]
    elif kind == 1:  # berry
        boxes = [(2, 4, 9, 15), (6, 1, 14, 15), (1, 8, 14, 15)]
    else:  # wide and low
        boxes = [(0, 9, 8, 15), (4, 7, 12, 15), (8, 9, 15, 15)]
    # Every bush sways: the lobes lean a pixel, which is the smallest movement the
    # eye reads as wind rather than as a different bush.
    mass = _leaf_mass([(b[0] + (sway if b[2] > 10 else 0), b[1] + (sway if b[2] > 12 else 0), b[2], b[3]) for b in boxes])
    img.alpha_composite(mass)
    # Leaf clusters on the lit shoulder, and a shadow where one lobe meets another:
    # the difference between a shrub and a green cushion.
    for cx, cy in ((5, 8), (9, 6), (7, 11), (11, 10)):
        if img.getpixel((cx, cy))[3]:
            put(img, cx, cy, LEAF.hi)
            put(img, cx + 1, cy, LEAF.lit)
            put(img, cx, cy + 1, LEAF.lit)
    if kind == 1:
        for x, y in ((5 + sway, 7), (9, 9), (11 + sway, 5), (4, 11)):
            put(img, x, y, CORAL.mid)
            put(img, x, y - 1, CORAL.hi if y > 6 else CORAL.lit)
            put(img, x + 1, y, CORAL.sh)
    # A darker line under the canopy: the shadow the mass casts on itself, not an outline.
    for x in range(1, 15):
        if img.getpixel((x, 15))[3]:
            put(img, x, 15, LEAF.deep)
    return img


def tuft(shape: int, frame: int) -> Image.Image:
    """Grass, straw and turf: blades growing out of the cell's floor."""
    sway = 1 if frame else 0
    img = canvas()
    if shape in (0, 1):  # fresh blades
        blades = ((2, 9, -1), (3, 11, 0), (5, 6, 0), (6, 8, 1), (8, 10, 0),
                  (10, 5, 1), (11, 8, 1), (13, 10, -1), (14, 12, 0))
        ramp = LEAF
    elif shape in (2, 3):  # dry straw
        blades = ((2, 6, -2), (3, 9, -1), (5, 3, 0), (6, 7, 0), (8, 5, 1),
                  (9, 10, 1), (11, 2, 1), (12, 6, 2), (14, 9, 1))
        ramp = STRAW
    else:  # dense turf
        blades = ((1, 11, 0), (2, 9, -1), (4, 10, 0), (5, 7, 1), (7, 9, 0),
                  (8, 11, 0), (10, 8, 1), (11, 10, 0), (13, 9, 0), (14, 11, 0))
        ramp = LEAF_DARK
    for x, height, lean in blades:
        top = height
        for y in range(top, 16):
            # A blade is one pixel wide and *leans*: the offset grows down the blade, so
            # a tuft is a fan rather than a picket fence.
            xx = x + round(lean * (y - top) / max(1, 15 - top)) + (sway if lean > 0 else 0)
            put(img, xx, y, ramp.mid if y > top else ramp.lit)
        put(img, x + (sway if lean > 0 else 0), top, ramp.hi)
    # The base: a pixel of shadow where the blades meet the ground, so a tuft is planted.
    for x in range(1, 15):
        if img.getpixel((x, 15))[3]:
            put(img, x, 15, ramp.deep)
    return img


def flowers(kind: int) -> Image.Image:
    """A patch of flowers: stems, leaves and three blossoms, standing on the floor."""
    img = canvas()
    petal = (GOLD, PINK, CREAM)[kind % 3]
    for x, top in ((4, 8), (8, 6), (11, 9)):
        for y in range(top, 16):
            put(img, x, y, LEAF.sh if y % 2 else LEAF.mid)
        put(img, x - 1, top + 3, LEAF.lit)
        put(img, x + 2, top + 2, LEAF.lit)
        # A five-petal blossom: wide enough to read at one tile, small enough that a
        # patch of three is still a patch rather than a tree.
        rect(img, x - 1, top - 3, x + 1, top - 1, petal.mid)
        put(img, x, top - 3, petal.hi)
        put(img, x - 1, top - 1, petal.sh)
        put(img, x + 1, top - 1, petal.sh)
        put(img, x, top - 4, petal.hi)
        put(img, x, top - 2, GOLD.lit)
    return img


def rock(big: bool) -> Image.Image:
    """A boulder and a pebble: the same material at two sizes."""
    img = canvas()
    if big:
        mask = _union([(2, 8, 13, 15), (1, 10, 14, 15)])
    else:
        mask = _union([(5, 12, 10, 15)])
    shaded = _shade_union(mask, STONE_PROP, speckle=False)
    img.alpha_composite(shaded)
    if big:
        put(img, 4, 10, STONE.hi)
        put(img, 8, 13, STONE.sh)
        put(img, 11, 11, STONE.lit)
    return img


def mushroom(pair: bool) -> Image.Image:
    img = canvas()
    spots = [(7, 8), (4, 11)] if pair else [(8, 9)]
    for cx, cap in spots:
        for y in range(cap, 16):
            put(img, cx, y, CREAM.mid)
            put(img, cx + 1, y, CREAM.sh)
        for x in range(cx - 3, cx + 4):
            put(img, x, cap, CORAL.mid)
        for x in range(cx - 2, cx + 3):
            put(img, x, cap - 1, CORAL.lit)
        put(img, cx - 3, cap, CORAL.sh)
        put(img, cx + 3, cap, CORAL.sh)
        put(img, cx - 1, cap - 1, CREAM.hi)
        put(img, cx + 1, cap - 2, CORAL.hi)
    return img


def log() -> Image.Image:
    """A fallen trunk: a round back, bark along the sides, and the sawn end facing us."""
    img = canvas()
    # The body is a cylinder, so its top is lit and its underside is in shade.
    for x in range(1, 15):
        put(img, x, 9, BARK.lit)
        for y in range(10, 15):
            put(img, x, y, BARK.mid if y < 13 else BARK.sh)
        put(img, x, 15, BARK.deep)
    # Bark: short vertical cracks, not a row of dots.
    for x in (3, 6, 9):
        for y in range(10, 14):
            put(img, x, y, BARK.sh)
    # The sawn end: a pale disc with concentric rings, standing proud of the bark.
    for y in range(9, 16):
        for x in range(10, 15):
            dx, dy = x - 12, y - 12
            if dx * dx + dy * dy <= 9:
                tone = WOOD.lit if abs(dx) + abs(dy) < 2 else WOOD.mid
                if (dx, dy) in ((0, 0), (1, 1), (-1, 1)):
                    tone = WOOD.sh
                put(img, x, y, tone)
    put(img, 12, 12, WOOD.sh)
    put(img, 11, 12, WOOD.hi)
    # Moss along the top, and a couple of shoots.
    for x in (2, 3, 4, 7, 8):
        put(img, x, 8, LEAF.mid)
        put(img, x, 9, LEAF.lit)
    return img


def stump() -> Image.Image:
    """A cut trunk: a jagged rim, rings on the face, and roots at the base."""
    img = canvas()
    for y in range(7, 15):
        for x in range(4, 12):
            tone = BARK.mid
            if x == 4:
                tone = BARK.lit
            elif x >= 10:
                tone = BARK.sh
            put(img, x, y, tone)
    # Jagged break: the rim is not a straight line.
    for x in range(4, 12):
        top = 6 + (1 if x in (5, 8) else 0)
        for y in range(top, 7):
            put(img, x, y, BARK.sh)
    # The sawn face.
    for x in range(5, 11):
        put(img, x, 6, WOOD.lit)
        put(img, x, 7, WOOD.mid)
    put(img, 7, 6, WOOD.sh)
    put(img, 8, 6, WOOD.sh)
    put(img, 6, 7, WOOD.hi)
    # Roots: two flares at the floor, and moss on the lit side.
    rect(img, 3, 14, 4, 15, BARK.sh)
    rect(img, 11, 14, 12, 15, BARK.deep)
    rect(img, 4, 15, 11, 15, BARK.deep)
    put(img, 4, 9, LEAF.mid)
    put(img, 5, 9, LEAF.lit)
    for x in (5, 8):
        put(img, x, 11, BARK.sh)
    return img


def reeds(frame: int) -> Image.Image:
    img = canvas()
    sway = 1 if frame else 0
    for x, top, lean in ((3, 6, -1), (6, 3, 0), (9, 5, 1), (12, 8, 1)):
        for y in range(top, 16):
            put(img, x + round(lean * (y - top) / (16 - top)) + sway * lean, y, LEAF.mid)
        head_x = x + lean + sway * lean
        for y in range(top - 2, top + 1):
            put(img, head_x, y, BARK.mid if y < top - 1 else BARK.lit)
        put(img, head_x + 1, top - 2, BARK.sh)
    for x in (2, 5, 8, 11, 14):
        put(img, x, 15, LEAF.sh)
    return img


def hanging_vine(frame: int) -> Image.Image:
    """A vine growing out of the ceiling: attached on row 0, reaching past halfway."""
    img = canvas()
    sway = 1 if frame else 0
    for y in range(16):
        x = 7 + round(0.8 * math.sin(y / 4.0 + (0.7 if frame else 0)))
        put(img, x, y, LEAF.mid)
        put(img, x + 1, y, LEAF.sh)
        if y % 3 == 0:
            put(img, x, y, LEAF.lit)
    for y, side in ((2, -1), (5, 1), (8, -1), (11, 1), (14, -1)):
        x = 7 + round(0.8 * math.sin(y / 4.0 + (0.7 if frame else 0)))
        for i in range(2):
            put(img, x + side * (i + 1), y - i, LEAF.lit if i == 0 else LEAF.mid)
        put(img, x + side * 3, y - 1, LEAF.hi)
    put(img, 7, 0, LEAF.hi)
    put(img, 8, 0, LEAF.lit)
    return img


def lantern() -> Image.Image:
    img = canvas()
    rect(img, 7, 6, 8, 15, IRON.sh)
    rect(img, 7, 6, 7, 15, IRON.mid)
    rect(img, 5, 14, 10, 15, IRON.deep)
    # Housing and a warm pane.
    rect(img, 4, 3, 11, 8, IRON.mid)
    rect(img, 5, 4, 10, 7, GOLD.mid)
    rect(img, 6, 5, 9, 6, GOLD.hi)
    rect(img, 4, 2, 11, 2, IRON.sh)
    put(img, 7, 1, IRON.mid)
    put(img, 8, 1, IRON.sh)
    put(img, 4, 3, IRON.hi)
    put(img, 11, 8, IRON.deep)
    # The glow, a soft diamond around the pane.
    for x, y in ((3, 5), (12, 5), (7, 9), (8, 9)):
        put(img, x, y, tint(GOLD.hi, 90))
    return img


def crystal() -> Image.Image:
    img = canvas()
    for x, base, height in ((4, 15, 8), (8, 15, 11), (11, 15, 6)):
        for y in range(base - height, base + 1):
            put(img, x, y, CYAN.mid)
            put(img, x + 1, y, CYAN.sh)
        put(img, x, base - height, CYAN.hi)
        put(img, x + 1, base - height + 1, CYAN.lit)
        put(img, x, base, CYAN.deep)
    # A glint, and a facet: the two pixels that make it a crystal rather than a spike.
    put(img, 8, 6, WHITE)
    put(img, 7, 8, CYAN.lit)
    for x in (3, 6, 10, 13):
        put(img, x, 15, CYAN.deep)
    return img


def signpost() -> Image.Image:
    img = canvas()
    rect(img, 7, 6, 8, 15, BARK.mid)
    rect(img, 7, 6, 7, 15, BARK.lit)
    rect(img, 2, 4, 12, 8, WOOD.mid)
    rect(img, 2, 4, 12, 4, WOOD.lit)
    rect(img, 2, 8, 12, 8, WOOD.sh)
    rect(img, 2, 4, 2, 8, WOOD.hi)
    # An arrow cut into the board.
    for i in range(4):
        put(img, 5 + i, 6, WOOD.deep)
        put(img, 5 + i, 6 - i, WOOD.deep)
    put(img, 10, 6, WOOD.deep)
    return img


def crate() -> Image.Image:
    img = canvas()
    rect(img, 3, 4, 12, 15, WOOD.mid)
    rect(img, 3, 4, 12, 4, WOOD.hi)
    rect(img, 3, 5, 12, 5, WOOD.lit)
    rect(img, 3, 15, 12, 15, WOOD.deep)
    for y in (8, 12):
        rect(img, 3, y, 12, y, WOOD.sh)
    for x in (3, 12):
        rect(img, x, 4, x, 15, IRON.sh)
    rect(img, 3, 15, 12, 15, IRON.deep)
    for i in range(8):
        put(img, 5 + i, 7 + i, IRON.mid)
    put(img, 4, 6, IRON.hi)
    return img


def banner(frame: int) -> Image.Image:
    img = canvas()
    sway = frame
    rect(img, 3, 0, 4, 15, BARK.mid)
    rect(img, 3, 0, 3, 15, BARK.lit)
    rect(img, 3, 0, 12, 1, IRON.mid)
    for y in range(2, 11):
        inset = (sway if y % 3 == 0 else 0)
        for x in range(5 + inset, 12 + inset):
            if y == 2:
                tone = CORAL.hi
            elif y >= 9:
                tone = CORAL.sh
            elif x >= 10 + inset:
                tone = CORAL.lit
            else:
                tone = CORAL.mid
            put(img, x, y, tone)
    # A gold chevron on the field.
    for i in range(3):
        put(img, 7 + i, 5 + i, GOLD.hi)
        put(img, 10 - i, 5 + i, GOLD.mid)
    put(img, 8, 8, GOLD.mid)
    return img


def tree(shape: int) -> tuple[Image.Image, Image.Image]:
    """A whole tree drawn on 16x32 and cut into the two cells it is drawn in."""
    big = Image.new("RGBA", (16, 32), CLEAR)
    trunk = 3 if shape == 0 else 2
    for y in range(22, 32):
        for x in range(7, 7 + trunk):
            big.putpixel((x, y), BARK.mid if x < 7 + trunk - 1 else BARK.sh)
        big.putpixel((7, y), BARK.lit)
    # Roots.
    for x in (6, 7, 8, 9):
        big.putpixel((x, 31), BARK.deep)
    if shape == 0:
        boxes = [(1, 3, 14, 20), (3, 0, 12, 14), (0, 9, 8, 22), (8, 8, 15, 22)]
    else:
        boxes = [(2, 4, 13, 19), (4, 1, 11, 13), (1, 10, 8, 22), (8, 9, 14, 22)]
    mask = Image.new("L", (16, 32), 0)
    draw = ImageDraw.Draw(mask)
    for box in boxes:
        draw.ellipse(box, fill=255)
    px = mask.load()

    def inside(x: int, y: int) -> bool:
        return 0 <= x < 16 and 0 <= y < 32 and px[x, y] > 0

    for y in range(32):
        for x in range(16):
            if not inside(x, y):
                continue
            if not inside(x, y - 1):
                tone = LEAF.hi if x < 9 else LEAF.lit
            elif not inside(x - 1, y):
                tone = LEAF.lit if y < 14 else LEAF.mid
            elif not inside(x, y + 1) or not inside(x + 1, y):
                tone = LEAF.sh if y > 5 else LEAF.mid
            elif y > 19 and (x + y) % 3 == 0:
                tone = LEAF.sh
            else:
                tone = LEAF.mid if (x * 5 + y * 3) % 9 else LEAF.lit
            big.putpixel((x, y), tone)
    # The two halves have to meet: whatever columns are canopy on the upper half's last
    # row are canopy on the lower half's first row too. Enforced rather than hoped for,
    # because an ellipse boundary landing exactly on the cut is a one-pixel hole through
    # the trunk of every tree in the game.
    for x in range(16):
        if big.getpixel((x, 15))[3] and not big.getpixel((x, 16))[3]:
            big.putpixel((x, 16), big.getpixel((x, 15)))
    # Leaf clusters: a few two-pixel highlights on the lit shoulder and a shadow
    # under each lobe. A canopy shaded only at its silhouette is a green balloon;
    # these are what make it read as leaves.
    for cx, cy in ((5, 7), (10, 11), (4, 15), (9, 5)):
        for dx, dy in ((0, 0), (1, 0), (0, 1)):
            x, y = cx + dx, cy + dy
            if big.getpixel((x, y))[3]:
                big.putpixel((x, y), LEAF.hi)
    for cx, cy in ((8, 18), (12, 16), (3, 19)):
        if big.getpixel((cx, cy))[3]:
            big.putpixel((cx, cy), LEAF.sh)
    base = Image.new("RGBA", (16, 16), CLEAR)
    top = Image.new("RGBA", (16, 16), CLEAR)
    base.alpha_composite(big.crop((0, 16, 16, 32)))
    top.alpha_composite(big.crop((0, 0, 16, 16)))
    return base, top


HAZE = type(CORAL)("#A8C0C6", "#8CA6AE", "#738E98", "#5C7A84", "#47636E")


def pine(shape: int, near: bool = False, hazier: bool = False) -> Image.Image:
    """A conifer: jagged skirts tapering to a point, over a visible trunk.

    The near rank is the one place a trunk reads as bark rather than as more foliage,
    so its two centre columns are the faded bark tone at the cell's floor — the same
    fact `atlas::tests::the_near_pines_have_brown_trunks` asks of the pixels.

    The silhouette is the whole job at this size: a fir is a cone with *notched* edges,
    not a stack of rectangles. Every tier flares a pixel at a time and ends in branch
    tips that reach past the row above, which is what stops the tree-line reading as a
    row of green triangles.
    """
    img = canvas()
    if near:
        ramp = type(CORAL)("#86A87E", "#6E9570", "#587E62", "#446653", "#32513F")
    elif hazier:
        ramp = HAZE
    else:
        ramp = LEAF_DARK
    trunk = rgba("#6C5848") if near else ramp.deep
    faded = (108, 88, 72, 255)
    # Trunk first, so every skirt overlaps it cleanly. Two columns, lit on the left.
    for y in range(11, 16):
        put(img, 7, y, trunk)
        put(img, 8, y, rgba("#5A4838") if near else ramp.deep)
    # Four tiers, each a triangle with a stepped, notched edge. `span` is the half
    # width at the tier's base; the next tier starts wider and lower.
    tiers = ((1, 4, 2), (3, 7, 4), (6, 10, 5), (9, 13, 6)) if shape == 0 else (
        (0, 4, 3), (2, 7, 4), (5, 10, 6), (8, 13, 7))
    for top, bottom, span in tiers:
        height = bottom - top
        for row in range(height + 1):
            y = top + row
            if y > 15:
                break
            half = 1 + round((span - 1) * row / max(1, height))
            # Notches: every second row lets the branch tips out one pixel further, so
            # the edge is a saw rather than a slope.
            if row % 2 == 1:
                half += 1
            x0, x1 = 8 - half, 8 + half - 1
            for x in range(x0, x1 + 1):
                if x <= x0 + 1:
                    tone = ramp.hi if row < 2 else ramp.lit
                elif x >= x1 - 1:
                    tone = ramp.sh
                else:
                    tone = ramp.mid
                put(img, x, y, tone)
            # The underside of the tier, in the material's darkest tone: a fir has a
            # shadow under every skirt, and that is what gives it layers.
            if row == height:
                for x in range(x0, x1 + 1):
                    if x % 3 != 1:
                        put(img, x, y, ramp.deep)
            # The pointed leader at the top of each tier.
            if row == 0:
                put(img, 8, y, ramp.hi)
                put(img, 7, y, ramp.lit)
    if near:
        for y in (14, 15):
            put(img, 7, y, faded)
            put(img, 8, y, faded)
    return img


def cloud(shape: int) -> Image.Image:
    """A cumulus with a shaded underside and a dithered edge.

    The first pass was four near-white tones and a rim-light, so every cloud was a
    white cut-out. This shades by *height within the column* instead: white crowns,
    a pale body, then blue-grey shadows under each lobe, with a checker dither at the
    boundaries. The base stays flat and at row 12 — the view anchors a cloud by it.
    """
    # Every shape keeps a pixel of air at the cell's sides: a cloud whose mask runs to
    # the edge is cropped flat by the sprite boundary, and a cumulus with a vertical
    # wall on one side is the one silhouette a cloud cannot have.
    boxes = [
        [(1, 6, 8, 12), (3, 3, 11, 12), (7, 5, 14, 12), (5, 1, 10, 6)],
        [(1, 8, 7, 12), (3, 6, 11, 12), (8, 8, 14, 12), (6, 4, 11, 8)],
        [(2, 7, 9, 12), (7, 6, 13, 12), (4, 4, 9, 8)],
    ][shape % 3]
    mask = _union(boxes)
    img = canvas()
    px = mask.load()
    crown, lit, mid = WHITE, rgba("#F1F8FB"), rgba("#DCEAF2")
    shade, deep = rgba("#C2D8E7"), rgba("#ADC7DA")
    for x in range(16):
        top = next((y for y in range(16) if px[x, y]), None)
        if top is None:
            continue
        for y in range(top, 13):
            if not px[x, y]:
                continue
            depth = y - top
            if y >= 11:
                tone = deep if (x + y) % 2 else shade
            elif depth <= 1:
                tone = crown
            elif depth <= 3:
                tone = lit
            elif y >= 9:
                tone = mid if (x + y) % 2 else shade
            elif depth <= 6:
                tone = mid
            else:
                tone = lit if (x + y) % 2 else mid
            put(img, x, y, tone)
    return img


def dust(frame: int) -> Image.Image:
    """The puff a running foot kicks up, low in the cell, soft-edged."""
    img = canvas()
    spread = 1 if frame else 0
    for box, alpha in (((1, 12, 5, 15), 210), ((5, 10, 10, 15), 230), ((9 + spread, 12, 14, 15), 190)):
        draw = ImageDraw.Draw(img)
        draw.ellipse(box, fill=rgba("#EADFC8", alpha))
    for x, y in ((4, 13), (8, 12), (11, 13)):
        put(img, x, y, rgba("#FFF8E8"))
    return img


def puff() -> Image.Image:
    """The burst a dying agent leaves: a warm cloud of dust, not a red boulder.

    It is drawn as concentric, overlapping lobes with lower alpha at the edges, so it
    reads as something dispersing. A corpse's colour on the burst made it look like a
    second, smaller enemy; dust is the thing the impact actually throws up.
    """
    img = canvas()
    outer, body, core = rgba("#C9B79F"), rgba("#E7D8C0"), rgba("#FFF5DF")
    for box, colour, alpha in (
        ((1, 8, 8, 15), outer, 120),
        ((8, 8, 15, 15), outer, 120),
        ((4, 3, 11, 11), outer, 130),
        ((3, 6, 9, 13), body, 205),
        ((7, 5, 12, 11), body, 215),
        ((6, 8, 10, 12), core, 235),
    ):
        ImageDraw.Draw(img).ellipse(box, fill=tint(colour, alpha))
    # Specks thrown clear of the main mass: the difference between a puff and a ball.
    for x, y in ((1, 6), (14, 6), (2, 12), (13, 12), (6, 2), (11, 2)):
        put(img, x, y, tint(outer, 150))
    return img


def spark(frame: int) -> Image.Image:
    """The sparkle at the goal: a four-point star, the second frame larger."""
    img = canvas()
    length = 6 if frame else 4
    cx, cy = 8, 8
    for i in range(1, length + 1):
        colour = GOLD.hi if i == length else GOLD.lit
        put(img, cx + i, cy, colour)
        put(img, cx - i, cy, colour)
        put(img, cx, cy + i, colour)
        put(img, cx, cy - i, colour)
    for dx, dy in ((1, 1), (-1, 1), (1, -1), (-1, -1)):
        put(img, cx + dx, cy + dy, GOLD.mid)
    put(img, cx, cy, WHITE)
    return img


# --------------------------------------------------------------------------------------
# The window's glyphs
# --------------------------------------------------------------------------------------

GLYPH_INK = rgba("#243A52")
GLYPH_LIGHT = rgba("#4F6C86")


def _glyph() -> Image.Image:
    return canvas()


def ui_glyph(name: str) -> Image.Image:
    img = _glyph()
    draw = ImageDraw.Draw(img)
    dark, light = GLYPH_INK, GLYPH_LIGHT
    if name == "ui_undo":
        draw.arc((3, 4, 13, 14), 300, 200, fill=dark, width=2)
        draw.polygon([(1, 5), (6, 2), (6, 8)], fill=dark)
        put(img, 4, 4, light)
        put(img, 5, 4, light)
    elif name == "ui_redo":
        undo = ui_glyph("ui_undo")
        img = undo.transpose(Image.Transpose.FLIP_LEFT_RIGHT)
    elif name == "ui_fit":
        # Four corner brackets with room between them: a frame, not a closed box.
        for x, y, sx, sy in ((1, 1, 1, 1), (14, 1, -1, 1), (1, 14, 1, -1), (14, 14, -1, -1)):
            draw.line([(x, y), (x + sx * 4, y)], fill=dark, width=2)
            draw.line([(x, y), (x, y + sy * 4)], fill=dark, width=2)
            put(img, x + sx, y + sy, light)
            put(img, x + sx * 2, y + sy, light)
    elif name == "ui_info":
        draw.ellipse((2, 2, 11, 11), outline=dark, width=2)
        draw.line([(9, 9), (14, 14)], fill=dark, width=3)
        put(img, 5, 4, light)
        put(img, 4, 5, light)
    elif name == "ui_restart":
        draw.arc((2, 3, 13, 14), 300, 250, fill=dark, width=2)
        draw.polygon([(11, 1), (15, 4), (10, 6)], fill=dark)
        put(img, 5, 4, light)
    elif name == "ui_pause":
        rect(img, 4, 3, 6, 13, dark)
        rect(img, 10, 3, 12, 13, dark)
        for y in range(3, 13):
            put(img, 4, y, light)
            put(img, 10, y, light)
    elif name == "ui_play":
        draw.polygon([(4, 2), (13, 8), (4, 14)], fill=dark)
        draw.polygon([(5, 4), (11, 8), (5, 12)], fill=light)
    elif name == "ui_step":
        draw.polygon([(2, 3), (9, 8), (2, 13)], fill=dark)
        rect(img, 11, 3, 13, 13, dark)
        for y in range(3, 13):
            put(img, 11, y, light)
    elif name == "ui_medal":
        rect(img, 5, 1, 7, 6, GOLD.mid)
        rect(img, 9, 1, 11, 6, CORAL.mid)
        put(img, 5, 1, GOLD.hi)
        put(img, 9, 1, CORAL.hi)
        draw.ellipse((3, 5, 12, 14), fill=GOLD.mid)
        draw.ellipse((4, 6, 10, 12), fill=GOLD.lit)
        put(img, 5, 7, GOLD.hi)
        put(img, 7, 11, GOLD.sh)
        # The little star stamped on it.
        put(img, 7, 8, GOLD.sh)
        put(img, 8, 8, GOLD.sh)
        put(img, 7, 9, GOLD.sh)
        put(img, 8, 9, GOLD.hi)
    elif name == "ui_mirror":
        # Two solid halves pointing away from a dashed axis: the document flipped
        # left to right. The axis is dashed so it reads as a mirror line rather than
        # as a third arrow.
        for y in range(1, 15, 3):
            put(img, 7, y, dark)
            put(img, 8, y, light)
        draw.polygon([(1, 3), (6, 8), (1, 13)], fill=dark)
        draw.polygon([(14, 3), (9, 8), (14, 13)], fill=light)
        put(img, 2, 4, light)
        put(img, 13, 4, dark)
    else:
        put(img, 7, 7, dark)
    return img
