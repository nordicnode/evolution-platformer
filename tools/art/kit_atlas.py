"""Assemble the shipped atlas: every named cell, in one fixed order.

`tools/gen_tileset.py` calls [`build`] and writes the PNG and manifest. The order here
is the *manifest's* order, grouped by family so the preview reads like a palette, and
every name the renderer can ask for appears exactly once.
"""

from __future__ import annotations

from PIL import Image

from . import kit, kit_cast, kit_props

EVEN = (0, 2, 4, 6, 8, 10, 12, 14)

# The seam families the renderer ships: every look and side except the two a level
# cannot ask for (a surface tile never has a crown under it).
SEAM_LOOKS = ("grass", "dirt", "stone", "beach", "packed", "sandstone", "brick", "masonry", "wood", "timber")
MISSING_SEAMS = {("grass", "b"), ("beach", "b")}


def sprites() -> list[tuple[str, Image.Image]]:
    out: list[tuple[str, Image.Image]] = []

    def add(name: str, image: Image.Image) -> None:
        out.append((name, image))

    add("empty", kit.canvas())

    # Soil, and the rock under it.
    for mask in range(16):
        add(f"ground_{mask}", kit.material_tile("grass", mask))
    add("ground_1_b", kit.surface_variant("grass", 0))
    add("ground_1_c", kit.surface_variant("grass", 1))
    for mask in EVEN:
        add(f"deep_{mask}", kit.material_tile("stone", mask))
    add("deep_0_b", kit.rock_variant(0))
    add("deep_0_c", kit.rock_variant(1))

    # Sand, and the sandstone under a dune.
    for mask in range(16):
        add(f"sand_{mask}", kit.material_tile("sand", mask))
    add("sand_1_b", kit.surface_variant("sand", 0))
    add("sand_1_c", kit.surface_variant("sand", 1))
    for mask in EVEN:
        add(f"sandstone_{mask}", kit.material_tile("sandstone", mask))

    # Brick, and dressed masonry under it.
    for mask in range(16):
        add(f"brick_{mask}", kit.material_tile("brick", mask))
    add("brick_1_b", kit.surface_variant("brick", 0))
    add("brick_1_c", kit.surface_variant("brick", 1))
    for mask in EVEN:
        add(f"masonry_{mask}", kit.material_tile("masonry", mask))

    # Timber decking, and the structural beams under it.
    for mask in range(16):
        add(f"wood_{mask}", kit.material_tile("wood", mask))
    add("wood_1_b", kit.surface_variant("wood", 0))
    add("wood_1_c", kit.surface_variant("wood", 1))
    for mask in EVEN:
        add(f"timber_{mask}", kit.material_tile("timber", mask))

    add("spike", kit.spike())
    add("spring", kit.spring())

    # Fluids: one picture four tiles square, cut into the cells the renderer samples.
    for index, cell in enumerate(kit.fluid_block("water")):
        add(f"water_b{index}", cell)
    add("water", kit.fluid_block("water")[0])
    add("water_top_a", kit.fluid_surface("water", 0))
    add("water_top_b", kit.fluid_surface("water", 1))
    add("water_bed", kit.fluid_bed("water"))
    add("water_wall_l", kit.fluid_wall("water", "l"))
    add("water_wall_r", kit.fluid_wall("water", "r"))

    for index, cell in enumerate(kit.fluid_block("lava")):
        add(f"lava_b{index}", cell)
    add("lava", kit.fluid_block("lava")[0])
    add("lava_top_a", kit.fluid_surface("lava", 0))
    add("lava_top_b", kit.fluid_surface("lava", 1))
    add("lava_bed", kit.fluid_bed("lava"))
    add("lava_wall_l", kit.fluid_wall("lava", "l"))
    add("lava_wall_r", kit.fluid_wall("lava", "r"))
    add("lava_ball", kit.lava_ball())

    add("vine", kit.vine_tile())
    add("spike_ball", kit.spike_ball())

    # The cast.
    for frame in range(6):
        add(f"player_idle_{'abcdef'[frame]}", kit_cast.player("idle", frame))
    for frame in range(12):
        add(f"player_run_{frame + 1}", kit_cast.player("run", frame))
    for frame in range(6):
        add(f"player_jump_{frame + 1}", kit_cast.player("jump", frame))
    for frame in range(6):
        add(f"player_fall_{frame + 1}", kit_cast.player("fall", frame))
    for frame in range(4):
        add(f"player_land_{frame + 1}", kit_cast.player("land", frame))
    add("player_hurt", kit_cast.player("hurt"))

    for index, suffix in enumerate("abcd"):
        add(f"walker_{suffix}", kit_cast.walker(index))
    add("walker_dead", kit_cast.walker_dead())
    for index, suffix in enumerate("abcd"):
        add(f"chaser_{suffix}", kit_cast.chaser(index))
    add("chaser_dead", kit_cast.chaser_dead())
    add("flyer_a", kit_cast.flyer(0))
    add("flyer_b", kit_cast.flyer(1))
    add("turret_r", kit_cast.turret())
    add("turret_l", kit_cast.turret().transpose(Image.Transpose.FLIP_LEFT_RIGHT))
    add("shot", kit_cast.shot())

    add("goal_top", kit_cast.goal_top())
    add("goal_bottom", kit_cast.goal_bottom())
    for frame in range(4):
        add(f"goal_flag_{frame + 1}", kit_cast.goal_flag(frame))
    add("spawn_marker", kit_cast.spawn_marker())

    # Scenery, in the order the view's dressing table names it.
    for index, suffix in enumerate("abcdef"):
        add(f"bush_{suffix}", kit_props.bush(index, index % 2))
    for index, suffix in enumerate("abcdef"):
        add(f"tuft_{suffix}", kit_props.tuft(index, index % 2))
    add("flowers", kit_props.flowers(0))
    add("flowers_pink", kit_props.flowers(1))
    add("flowers_white", kit_props.flowers(2))
    add("rock", kit_props.rock(True))
    add("pebble", kit_props.rock(False))
    add("mushroom", kit_props.mushroom(False))
    add("mushroom_pair", kit_props.mushroom(True))
    add("log", kit_props.log())
    add("stump", kit_props.stump())
    add("reeds_a", kit_props.reeds(0))
    add("reeds_b", kit_props.reeds(1))
    add("vine_a", kit_props.hanging_vine(0))
    add("vine_b", kit_props.hanging_vine(1))
    add("lantern", kit_props.lantern())
    add("crystal", kit_props.crystal())
    add("signpost", kit_props.signpost())
    add("crate", kit_props.crate())
    add("banner_a", kit_props.banner(0))
    add("banner_b", kit_props.banner(1))
    for shape, suffix in ((0, "a"), (1, "b")):
        base, top = kit_props.tree(shape)
        add(f"tree_{suffix}_base", base)
        add(f"tree_{suffix}_top", top)
    add("butterfly_a_1", kit_cast.butterfly(0, 0))
    add("butterfly_a_2", kit_cast.butterfly(1, 0))
    add("butterfly_b_1", kit_cast.butterfly(0, 1))
    add("butterfly_b_2", kit_cast.butterfly(1, 1))
    add("pine_a", kit_props.pine(0))
    add("pine_b", kit_props.pine(1))
    add("pine_c", kit_props.pine(0, hazier=True))
    add("pine_d", kit_props.pine(1, hazier=True))
    add("pine_near_a", kit_props.pine(0, near=True))
    add("pine_near_b", kit_props.pine(1, near=True))

    # Clouds and effects.
    add("cloud_a", kit_props.cloud(0))
    add("cloud_b", kit_props.cloud(1))
    add("cloud_c", kit_props.cloud(2))
    add("dust_a", kit_props.dust(0))
    add("dust_b", kit_props.dust(1))
    add("puff", kit_props.puff())
    add("spark_a", kit_props.spark(0))
    add("spark_b", kit_props.spark(1))

    # Platforms and machines.
    for part in ("l", "m", "r"):
        add(f"oneway_{part}", kit.oneway(part))
    for part in ("l", "m", "r"):
        add(f"mover_{part}", kit.mover(part))
    add("belt_r", kit.belt("r"))
    add("belt_l", kit.belt("l"))
    add("boulder", kit.boulder())
    add("block", kit.block())
    add("rope", kit.rope_icon())

    # The window's own glyphs.
    for name in ("ui_mirror", "ui_undo", "ui_redo", "ui_fit", "ui_info",
                 "ui_restart", "ui_pause", "ui_play", "ui_step", "ui_medal"):
        add(name, kit_props.ui_glyph(name))

    # Brittle-tile marks: three per material, drawn over the material's own tile.
    for material in ("ground", "sand", "brick", "wood"):
        add(f"cracked_{material}", kit.crack_overlay(material, 0))
        add(f"cracked_{material}_b", kit.crack_overlay(material, 1))
        add(f"cracked_{material}_c", kit.crack_overlay(material, 2))

    # Material seams, last because they are the only family drawn over other families.
    for look in SEAM_LOOKS:
        for side in ("l", "r", "t", "b"):
            if (look, side) in MISSING_SEAMS:
                continue
            add(f"blend_{look}_{side}", kit.seam_sprite(look, side))

    names = [name for name, _ in out]
    assert len(names) == len(set(names)), "a sprite name appears twice"
    return out


def build(cols: int = 16) -> tuple[Image.Image, list[dict]]:
    images = sprites()
    rows = (len(images) + cols - 1) // cols
    atlas = Image.new("RGBA", (cols * kit.CELL, rows * kit.CELL), kit.CLEAR)
    manifest = []
    for index, (name, image) in enumerate(images):
        col, row = index % cols, index // cols
        atlas.alpha_composite(image, (col * kit.CELL, row * kit.CELL))
        manifest.append({"name": name, "col": col, "row": row})
    return atlas, manifest
