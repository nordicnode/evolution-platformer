# Assets

Generated art. `tools/gen_tileset.py` rewrites everything in this directory:

    python3 tools/gen_tileset.py

| file | what it is |
|---|---|
| `tileset.png` | the atlas: 16x16 pixel cells, 8 columns by 3 rows (128x48) |
| `tileset.json` | the manifest the game reads: cell size, grid, and where each named sprite lives |
| `tileset_preview.png` | an 8x nearest-neighbour upscale on a sky background, for eyeballing |

The game reads `tileset.json`, not hard-coded coordinates, so art can be replaced
without touching Rust: draw a new atlas, list your sprites in the manifest, and any
name the game asks for that is missing is a startup error naming what it found.

## Sprites

| name | cell | used for |
|---|---|---|
| `empty` | (0,0) | nothing — fully transparent |
| `grass` | (1,0) | a solid tile with open air above it |
| `grass_left` | (2,0) | the same, with open air to its left |
| `grass_right` | (3,0) | the same, with open air to its right |
| `dirt` | (4,0) | a solid tile buried under another solid tile |
| `dirt_left` / `dirt_right` | (5,0) / (6,0) | buried, with open air on that side |
| `spike` | (7,0) | a lethal non-solid tile |
| `player_stand` | (0,1) | the player at rest |
| `player_run_a` / `player_run_b` | (1,1) / (2,1) | two run frames, alternated while moving |
| `player_jump` | (3,1) | the player in the air |
| `walker_a` / `walker_b` | (4,1) / (5,1) | two walker frames, alternated |
| `walker_dead` | (6,1) | a stomped walker |
| `goal_top` | (7,1) | the flag, drawn in the goal's upper tile |
| `goal_bottom` | (0,2) | the pole, drawn in the goal's lower tile |
| `spawn_marker` | (1,2) | the spawn point, editor only |

Tiles are picked by what surrounds them, so a level needs no per-tile art
decisions: a solid tile is `grass` when there is nothing above it and `dirt`
otherwise, with the `_left`/`_right` variants when a side is exposed.

## Replacing the art

The palette lives at the top of `tools/gen_tileset.py`; the characters are drawn
as 16-row string art further down, so editing a sprite means editing a picture.
Speckle texture comes from a hash-based value noise rather than `random`, so
regenerating never produces a spurious diff.
