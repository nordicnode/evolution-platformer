# Assets

Generated art. `tools/gen_tileset.py` rewrites everything in this directory:

    python3 tools/gen_tileset.py

| file | what it is |
|---|---|
| `tileset.png` | the atlas: 16x16 pixel cells, 16 columns by 10 rows (256x160) |
| `tileset.json` | the manifest the game reads: cell size, grid, and where each named sprite lives |
| `tileset_preview.png` | an 8x nearest-neighbour upscale on a checked background, for eyeballing |

The game reads `tileset.json`, not hard-coded coordinates, so art can be replaced
without touching Rust: draw a new atlas, list your sprites in the manifest, and any
name the game asks for that is missing is a startup error naming what it found.

The *drawing* code is one module per family under `tools/art/`, and every colour in
the game comes from `tools/art/common.py`, so terrain, characters and effects cannot
drift into three palettes. The table below is the manifest in words; it is checked
against the shipped manifest by
`atlas::tests::the_assets_readme_lists_every_shipped_sprite`, so it cannot go stale
quietly. Most names are **required** (`crate::atlas::REQUIRED`): the game refuses to
start without them. The tile variants, the last eight props and every material seam are
*optional* — a tileset that omits them draws the plainer equivalent instead (a hard edge,
for a missing seam), and the fallbacks are in `crate::view::scenery_sprite` and
`crate::view::draw_seams`.

## Sprites

| names | used for |
|---|---|
| `empty` | nothing — fully transparent |
| `ground_0`, `ground_1`, `ground_2`, `ground_3`, `ground_4`, `ground_5`, `ground_6`, `ground_7`, `ground_8`, `ground_9`, `ground_10`, `ground_11`, `ground_12`, `ground_13`, `ground_14`, `ground_15` | one solid soil tile per 4-bit "open to the air on this side" mask: bit 0 above, 1 left, 2 right, 3 below. The mask is computed from a tile's neighbours (`view::ground_mask`), so a level's silhouette needs no per-tile art. `ground_1` is the plain floor |
| `deep_0`, `deep_2`, `deep_4`, `deep_6`, `deep_8`, `deep_10`, `deep_12`, `deep_14` | the same construction in stone, for masks with no sky above (rock is never a surface), once a tile is three deep |
| `sand_0`, `sand_1`, `sand_2`, `sand_3`, `sand_4`, `sand_5`, `sand_6`, `sand_7`, `sand_8`, `sand_9`, `sand_10`, `sand_11`, `sand_12`, `sand_13`, `sand_14`, `sand_15` | the same sixteen masks in **sand**: a second material, painted with the same brush and solid in exactly the same way, drawn as a bright crown of loose sand over packed sand. `sand_1` is the plain beach |
| `sandstone_0`, `sandstone_2`, `sandstone_4`, `sandstone_6`, `sandstone_8`, `sandstone_10`, `sandstone_12`, `sandstone_14` | the rock under a beach, for masks with no sky above, with horizontal bedding planes instead of stone's vertical cracks |
| `ground_1_b`, `ground_1_c` | the floor tile plus flowers, and plus a tuft and a pebble |
| `deep_0_b`, `deep_0_c` | rock plus scattered pebbles, and rock plus a crack |
| `sand_1_b`, `sand_1_c` | the beach plus wind ripples, and plus a shell and a scatter of grit |
| `spike` | a lethal non-solid tile: three spikes on a stone base, top row transparent so it never looks like part of the ceiling |
| `player_idle_a`, `player_idle_b` | the runner at rest, breathing |
| `player_run_1`, `player_run_2`, `player_run_3`, `player_run_4`, `player_run_5`, `player_run_6` | the six-frame run cycle |
| `player_jump`, `player_fall`, `player_land`, `player_hurt` | rising, falling, the landing frame, and the pose a dead agent is left in |
| `walker_a`, `walker_b`, `walker_c`, `walker_d` | the patroller's four-frame walk |
| `walker_dead` | a stomped walker |
| `goal_top`, `goal_bottom`, `goal_flag_1`, `goal_flag_2`, `goal_flag_3`, `goal_flag_4` | the flag on its pole, waving through four frames |
| `spawn_marker` | where the player starts, editor only |
| `bush_a`, `bush_b` | a leafy mound, two frames of sway |
| `bush_c`, `bush_d` | a taller bush with berries on it, two frames of sway |
| `bush_e`, `bush_f` | a wide, low bush that still fits under a one-tile ceiling, two frames of sway |
| `tuft_a`, `tuft_b` | blades of grass, two frames of sway |
| `tuft_c`, `tuft_d` | long dry blades gone to straw, two frames of sway |
| `tuft_e`, `tuft_f` | short, dense turf, two frames of sway |
| `tree_a_base`, `tree_a_top`, `tree_b_base`, `tree_b_top` | two trees, each drawn whole and cut into two cells: `base` is the cell the trunk stands in and `top` the cell above it, and the two halves meet at a seam they agree on column for column |
| `flowers`, `flowers_pink`, `flowers_white` | patches of flowers in three colours |
| `rock`, `pebble` | a boulder and a small stone |
| `mushroom`, `mushroom_pair` | a capped mushroom, and a patch of two |
| `log`, `stump` | a fallen log with a ringed sawn end, and the stump it came from |
| `reeds_a`, `reeds_b` | cattails, two frames of sway |
| `vine_a`, `vine_b` | a vine hanging from a tile's underside, two frames of sway |
| `cloud_a`, `cloud_b`, `cloud_c` | three cumulus shapes for the parallax sky |
| `butterfly_a_1`, `butterfly_a_2`, `butterfly_b_1`, `butterfly_b_2` | two butterflies, two frames of wingbeat each — the only sprites in the game that are not standing on anything |
| `pine_a`, `pine_b` | two conifer silhouettes: the tree-line standing on the ridge of the far hills it is drawn in front of |
| `pine_c`, `pine_d` | the same two in the hazier palette of the rank behind: a forest edge, not a row |
| `pine_near_a`, `pine_near_b` | the same two again in the near hills' own tree-line green — the conifer hazed half way to the near hills' colour, with a grey-faded bark trunk: one band closer, so bigger, and optional art — a tileset without them has bare near hills |
| `dust_a`, `dust_b` | the puff a running foot kicks up |
| `puff` | the burst a dying agent leaves |
| `spark_a`, `spark_b` | the sparkle at the goal |
| `blend_grass_l`, `blend_grass_r`, `blend_grass_t` | a lawn's grass crossing the left/right/top edge of the tile beside it |
| `blend_dirt_l`, `blend_dirt_r`, `blend_dirt_t`, `blend_dirt_b` | the dirt under a lawn, same four edges |
| `blend_stone_l`, `blend_stone_r`, `blend_stone_t`, `blend_stone_b` | the rock under both, same four edges |
| `blend_beach_l`, `blend_beach_r`, `blend_beach_t` | a beach's sand crossing three of the four edges — never the bottom: see below |
| `blend_packed_l`, `blend_packed_r`, `blend_packed_t`, `blend_packed_b` | the packed sand three tiles into a dune, same four edges |
| `blend_sandstone_l`, `blend_sandstone_r`, `blend_sandstone_t`, `blend_sandstone_b` | the sandstone under a beach, same four edges |

**Scenery** — the bushes, tufts, flowers, rocks, mushrooms, logs, stumps, reeds, vines
and trees — is not part of the level format. The view scatters it along the surfaces of
solid tiles from a hash of the tile's own coordinates, so a level looks the same
every frame and after every regeneration, and a prop can never appear where the
physics disagrees with it. Surface props stand in the *open* tile above the ground
they are planted on; vines hang in the open tile below the solid one they grow from;
and a tree, which is the one prop taller than a tile, is planted only where every cell
its canopy reaches into is empty too — where there is no room it is a bush instead.

**A prop's cell is a slot, not a size.** Each kind is drawn at its own height in tiles,
measured up from the bottom of that cell, so the cell the art was drawn in says nothing about
how big the thing in it is: `view::prop_size` draws a tree as three tiles of canopy over a
trunk, a bush at three quarters of a tile and a daisy patch at a third, all standing on the
same ground line. The tree is the only prop drawn larger than the cell it stands in, which is
why it is the only one that needs the room above it checked; the rest are *smaller* than their
cell, which is what stops a patch of flowers from being as wide as a tree's canopy.

**Material** is part of the level format: `#` is soil and `S` is sand (`sim::Tile`).
They are the same tile to the physics and to an agent's senses, and they differ only in
which autotile family the view draws, so a level can be a beach without being a new
problem to solve. A tileset without `sand_*` draws a beach as soil.

**The seams between materials are generated, not painted per pair.** Where two materials
meet — a beach that begins in the middle of a lawn, a dune standing on soil, a sand column
running through rock — each one creeps a few ragged pixels into the other's tile, and the
view draws the *neighbouring* material's band over the tile beside it. A band is named for
the material and the side (`blend_{look}_{l|r|t|b}`) and never for a pair of materials: four
materials make twenty-two bands, in both directions, where a table of pairs would need four
more for every material already shipped to say the same thing twice. The six *looks* are
the two materials' three appearances each — `grass`/`dirt`/`stone` for soil and
`beach`/`packed`/`sandstone` for sand — and a band is only ever drawn between two different
*materials*, because a crown and the body under it already join: the crown's last row is
painted ragged against the body for exactly that reason.

Every band is a mask over the material's own plain tile, so its pixels *are* that material's
pixels and no seam can be a slightly-wrong colour; how far each one reaches is a property of
the material rather than of the pair (sand spills five pixels, soil three, sandstone three,
stone two), and every band is mostly transparent — the largest is 18% opaque — because a
band that covered its cell would be a stamp rather than a spill. A band is **a scatter of
grains rather than a solid wedge**: a row of the seam reaches nothing, one pixel, or all the
way in, and the grains inside a row it does reach thin out with depth, so the two materials
interlock along a wandering boundary instead of stacking into a stripe of each inside the
other. Nothing can leak sky either way — a band is drawn *over* the opaque tile it belongs
to — so the only question a seam answers is which of the two materials owns a pixel. `blend_grass_b` and
`blend_beach_b` are the two bands that do not exist, and the omission is structural rather
than an oversight: a surface tile has sky above it, so no solid tile ever has a crown
*below* it, and those two situations cannot be built in the level format. Like the tile
variants, a band is **optional** art: a tileset without them draws the hard edge every
tileset drew before they existed.

**Butterflies are not scenery.** They are the view's ambient layer
(`view::draw_ambient`), because a butterfly belongs to no tile: it is drawn in a tile
the level calls empty — never inside a wall — with open air above it and ground below
it, so they live along the grass line rather than all over the sky, and it drifts around
that tile's centre as a function of the clock. It is drawn a fifth of a tile across and
the pack holds twenty of them, three or four to a level. The editor shows none of them,
for the same reason it shows no sky: a document is about what the file says, and the file
says nothing about a butterfly.

## Replacing the art

The palette lives at the top of `tools/art/common.py`; each family is drawn in its own
module there (`terrain.py`, `characters.py`, `props.py`), mostly as 16-row string art
or small drawing helpers, so editing a sprite means editing a picture. Texture comes
from a hash-based value noise rather than `random`, so regenerating never produces a
spurious diff. Tiles are picked by what surrounds them, which is why the grid, not the
artist, decides which `ground_N` a given floor tile asks for.
