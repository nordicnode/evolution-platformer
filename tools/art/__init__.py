"""The game's pixel art, one module per family.

`tools/gen_tileset.py` imports these modules, flattens their `sprites()` lists in a
fixed order and writes `assets/tileset.png` plus the manifest the game reads. The
shared colours and the 16-row drawing helper are in `common.py`; a module owns the
pictures, the driver owns the grid.
"""
