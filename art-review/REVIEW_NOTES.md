# Review notes

## Iteration 3 — the shipped production pass

The study in this folder proposed a heavy navy outline on every object. It was
right about the shapes and wrong about the ink: at 16 pixels an outline on every
sprite is the single strongest prototype tell, and it also broke the terrain,
because a row of outlined cells is a row of stickers rather than a floor.

The shipped direction, and the one `tools/art/kit*.py` draws, is:

- **No object outline.** Form is carried by a lit rim, a mid face and a shadowed
  underside, using each material's own ramp — three to five tones, one warm light
  from the upper left.
- **Seamless terrain.** Soil, sand, brick, stone, masonry and timber are textures
  keyed to their own coordinates, so adjacent tiles continue each other. The
  autotile adds only the crown and the edge light.
- **Drawn animation.** The runner has six distinct run poses; the walker, chaser
  and flag each have four; bushes, reeds, vines, dust and sparks have two; every
  frame was redrawn rather than nudged. The art tests compare the shipped pixels.
- **A sky with form.** Clouds are shaded by height with a dithered underside, the
  pines have notched skirts and visible trunks, and the rocks are faceted.
- **Chrome that stays out of the way.** The dark course-bounds box is an editing
  affordance only; the play view no longer cuts a navy line across its own sky.

## Proposed personality

**One Two Hundred is a course-making toy box.** The player should feel like they
are arranging colorful play pieces, then cheering on a little explorer and its
AI friends. The proposed pass keeps the existing pixel-art discipline but makes
the shapes and colors more instantly legible at a glance.

## Iteration 2 correction (historical)

The first concept pass over-used the same heavy outline on terrain and on
objects. That made repeated ground cells look like a row of stickers. The
review set then applied the outline to movable/interactive objects and UI;
surface cells were opaque and edge-to-edge. Iteration 3 kept the terrain rule
and removed the object outline as well.

## What changes in the visual direction

| Area | Current tendency | Review direction |
|---|---|---|
| Silhouettes | Detailed 16px sprites with restrained contrast | Chunky dark outline, fewer interior pixels, stronger shape language |
| Terrain | Naturalistic texture and muted earth tones | Toy blocks with clear material colors and simple repeatable texture |
| Characters | Small cast that reads well in the world | More expressive faces, stronger pose differences, friendlier proportions |
| Hazards | Accurate mechanical objects | Big readable warning shapes with clear color families |
| Scenery | Quiet world dressing | Colorful course decoration that rewards looking without competing with the player |
| Menu | One title menu plus compact dock controls | Four large activity doors: Build, Play, Train, Replay |
| Editor | Dense palette organized as a long parts list | A toy tray organized as Build, Danger, Helpers, Friends, Finish |
| Training | Technical metrics around a live field | A stage, an explanation card, a progress meter, and celebratory status |
| Replay | Accurate record/ghost information | “Star run” framing with colored ghost paths and a simple timeline |

## Proposed UI rules

1. **Navy outline for objects and UI.** Cards, controls, characters, hazards,
   and movable pieces share the outline; terrain owns its own continuous seams.
2. **Yellow means attention.** Selection, play, reward, and the currently active
   mode use the same yellow face and hard lower shadow.
3. **Ribbons name spaces.** A colored ribbon says what a panel is for before its
   details need to be read.
4. **Cards hold one fact.** Course size, best time, solved count, and learning
   progress each get their own small card instead of competing in one paragraph.
5. **Technical detail stays available.** The child-friendly layer is a clearer
   front door, not a removal of the run’s real numbers.
6. **Use words with the pictures.** Build, Play, Train, and Replay remain visible
   labels; icons reinforce them rather than becoming a memory test.

## Promoted asset coverage

The generated board covers the same broad families the live game uses:

- ground materials and movable objects
- hazards, elements, ropes, belts, springs, and platforms
- player, enemies, goal, and spawn
- scenery and effects
- editor/run/replay glyphs

`alternate_tileset.json` records the curated concept sprite names, while
`complete_candidate_tileset.json` preserves every manifest name and coordinate
*at the time of this study*. The live atlas is generated separately by
`tools/gen_tileset.py` from `tools/art/kit*.py`; both candidate files here are
review-only, and the `09-complete-atlas.png` board shows the iteration-2 look,
not the current game.

## Implemented decisions

- The cap-and-scarf explorer is the live player identity.
- The fourth title door is **Replay**.
- AI uses purple for immediate recognition against the warm course palette.
- Green grass remains the default course theme; the existing material families
  remain available in Build mode.
