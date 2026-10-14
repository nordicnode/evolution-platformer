# Evolution Platformer — review and polish pass

**Date:** 2026-10-13
**Scope:** whole workspace (`sim`, `evo`, `render`, `train-cli`, `app`, `tools/art`).
**Method:** read the crates against `development_plan.md` and `README.md`, built
`--workspace --release`, ran the whole suite (`cargo test --workspace`), ran
`cargo clippy --workspace --all-targets`, rendered real frames from the app with
`--screenshot` and diffed the pixels, and commissioned two web-research passes
(neuroevolution technique + 2D game-feel/pixel-art/editor UX). Every claim about a
behaviour below was checked against the code or a rendered frame.

The codebase is in much better shape than "rough prototype": 413 tests green, no
clippy lints (`--all-targets`), and a genuinely unusual amount of
the *why* recorded in comments and findings. The polish gaps that remain are
concentrated in three places — the editor's palette and placement rules, the front
door, and the absence of any juice (audio, shake, hit-stop) — plus a handful of
real interaction bugs, five of which this pass fixes.

The five were all found the same way, and it is worth naming: **nothing in the suite
pressed a key or clicked a tile.** The app's tests build a `Frame`, an `Editor` and a
`Level` and assert on those, so a binding that fires twice, a toggle in the wrong
mode, or a click that addresses a cell the art is not drawn in stays green forever.
Four of the five are now held by tests that *do* click (through `Editor::paint`, which
is what the mouse handler calls).

---

## 1. Fixed in this pass

Each item ends with the test(s) that now hold it.

### 1.1 `f` (follow) did nothing in the window's Evolve and Replay views — **bug**

`crates/app/src/app.rs` read `F` once directly in the `Mode::Evolve` / `Mode::Replay`
branches *and* again inside [`panel::commands_for`] (which maps `Keys::follow` to
`Command::ToggleFollow`). Two toggles on one keypress cancelled out, so the
documented "`f` follow the leader / whole level" was a no-op in exactly the views
that document it. Fixed by deleting the duplicate handling; the key map stays the
single source of truth in `panel`. Held by every existing follow test plus the new
key-map coverage below.

### 1.2 `Ctrl+Shift+Z` (redo) fired undo *and* redo — **bug**

The undo predicate (`Z` + `command`) also matched `Ctrl+Shift+Z`, which the redo
predicate claims. Both ran, so the pair cancelled and the level never moved — the
standard redo chord was dead. Undo now excludes `shift`. (`Ctrl+Y` and
`Ctrl+Shift+Z` both redo.)

### 1.3 `G` toggled the grid in every mode — **bug**

`G` flipped `self.grid` unconditionally, but the grid is only *drawn* in `Mode::Edit`.
In the evolution view `G` also means "ghosts", so one press flipped the ghosts on
screen **and** the editor's grid silently behind them. The grid toggle now belongs to
the editor only.

### 1.4 Placed scenery — **asset variety**

The palette could place eight things (ground, sand, spike, clear, spawn, goal,
walker, erase), and every bush, tree, rock, flower and vine in the game was
*hash-scattered* by the renderer — visible, but not authorable. You could not put a
tree where you wanted one.

This adds a **scenery layer**: a `scenery` entity in the level file carrying an atlas
sprite name and a cell.

* `crates/sim/src/level.rs` — `Scenery` type, load/write, `validate`, `resize`
  (drops what falls outside), and the editing API (`add_scenery`,
  `scenery_at`, `remove_scenery_at`, `scenery_placeable`). Loader refuses a scenery
  entity with no sprite name.
* **Scenery is deliberately outside `content_hash`.** It is inert — the physics
  never sees it, sensors cannot detect it — so decorating a level must not invalidate
  a champion trained on it, and keeping it out of the hash means *every hash this
  build produced before scenery existed is still valid* (existing champions keep
  working; no version bump). Held by
  `scenery_round_trips_stays_inside_the_grid_and_is_not_hashed`.
* `crates/app/src/view.rs` — placed scenery is drawn over the terrain (so it is never
  hidden by the ground, unlike the hashed pass) via `placed_prop`, which reuses the
  existing `Prop` sizing and gives a `*_base` sprite its canopy, so a placed tree
  stands as tall as a scattered one. A name the tileset lacks draws nothing rather
  than a substitute. Held by `placed_scenery_is_drawn_in_the_frame` (bare frame +1
  quad for a bush, +2 for a tree, back to bare when cleared) — the same
  "is the thing I placed on the screen?" question finding 30 taught the suite to ask.
* `crates/app/src/editor.rs` — a ninth brush, `Brush::Scenery`, placing
  `Editor::scene_sprite`; a curated `SCENERY_SPRITES` list (17 props: bushes, tufts,
  three flowers, rock, pebble, two mushrooms, log, stump, reeds, vine, two trees);
  and the eraser now clears decoration **before** the terrain, so one click on a
  decorated tile takes the bush rather than the floor under it.
* `crates/app/src/app.rs` — a thumbnail picker appears under the palette while the
  scenery brush is selected, and the brush's palette icon *and* the drag ghost track
  the selection (`brush_icon`), so the menu never shows a bush while the brush plants
  a tree.

Nineteen new placeable props, no new art, no atlas change. Held by
`the_scenery_brush_places_its_sprite_and_the_eraser_takes_it_back`.

### 1.5 The front door now leads where its sign says — **splash**

The title screen printed `P play   E evolve` but *any* key just dismissed it into
the editor, and the dismissing press was consumed by the screen — so the named keys
did nothing. `Splash::show` now returns an `Action` (`Edit` / `Play` / `Evolve`) and
`EvoApp` enters that mode, so `P` opens the playtest and `E` starts a run. The
invitation reads "press any key to build a level / P play it yourself · E evolve an
AI", and a soft dark band behind it keeps it legible over any world rather than only
over clean haze. Held by `the_named_keys_open_the_activity_they_name` and the
existing dismissal test.

### 1.6 The eraser could not reach the art you can see — **bug, reported from play**

> "I am able to place a new goal, but I am unable to delete the one that is already
> placed."

A goal's marker is **two tiles tall**: `view::goal_cells` draws `goal_top` and the
waving cloth in the cell *above* the goal entity, and `goal_bottom` (the pole's foot)
in the entity's own cell. The editor addressed it by one cell — the lower one — so the
click everyone aims at, straight at the flag, landed on a tile no goal claimed and fell
through to `set_tile(…, Empty)` on a cell that was already empty. The click did
nothing, twice over, and the only way to delete a flag was to guess the tile *under*
the flag.

The same shape was in the scenery eraser: a `*_base` tree draws its canopy two cells
up (which is exactly why `scenery_placeable` makes it prove those cells clear), so a
click in the leaves removed nothing.

Fixed in `sim` rather than in the editor, because the document owns its rules here:

* `Level::goal_covers(tx, ty)` — a goal answers for **both** its cells. Every question
  that has to agree with what the player sees goes through it: `remove_goal_at`
  (so the eraser takes it by the flag *or* the pole, and the last-goal rule refuses
  either), `add_goal_tile` and the new `Level::goal_placeable` (so a flag cannot be
  stacked into another marker's cell — including the sneaky one, a goal whose *own*
  flag would hang in an existing goal's pole).
* `remove_scenery_at` — a piece is addressed by any cell of the art it fills: its own
  cell, or any canopy cell for a `_base` tree. A cell holding a piece of its own still
  wins over a canopy reaching over it, so the first click is always what is under the
  pointer.
* `Editor::can_place` answers for `Brush::Goal` too, so the drop ghost turns red
  *before* the click, and `paint_one` says why ("there is already a goal here") instead
  of silently doing nothing.
* `app.rs` — the drag ghost for the goal brush now draws the **whole marker**
  (pole + finial + a flag frame) and outlines both cells, so the ghost stops promising
the flag in a place the flag will not be. The ghost is the one part of this fix that
  no test drives, since it needs a live pointer drag.

Held by four tests, two in `sim` and two in `app`, plus a view test that makes the
correspondence explicit — `the_flag_is_drawn_in_the_cell_the_eraser_addresses` reads
the rendered frame's flag and pole quads back through the camera and asserts the two
cells they land in are the two `goal_covers` accepts. It fails if either half moves:
narrow `goal_covers` back to the anchor and the flag's cell stops answering; draw the
flag a cell lower and the read-back tile changes.

### 1.7 The flag on a new level could not be deleted, and a second one could be placed — **bug, reported from play**

> "The goal flag (that is originally placed at the very start of the level without the
> user placing it) still cannot be deleted. Also, there should not be more than 1 goal
> at any one time."

1.6 made the flag *addressable*, and the report it came from was about the flag you can
see. This one is the flag you cannot get rid of. `Level::remove_goal_at` refused to
remove the document's **last** goal, on the principle in the module docs — "the level
is never left in a state that cannot be saved", since every later save is rejected —
so the flag `Level::starter` plants at `(width-2, height-2)` of every new level was
permanently on the map, at every size, in every new document. The principle was right
and the rule was worse than the disease: the fix is not to lock the flag but to say
what a missing one costs. Beside that, `add_goal_tile` accepted a second flag anywhere
it fit, which is a level where "the goal" is a question the oracle, the sensors and
the score each have to guess at.

Both are one rule now, and it lives in the document:

* **A level has one flag.** `Level::add_goal_tile` became `Level::set_goal_tile`: a
  goal placed anywhere else *relocates* the marker, exactly as `set_spawn_tile` moves
  the one spawn. The two-cells-are-the-marker refusals from 1.6 still answer, so a
  click on the flag itself is refused with a message — "the goal is already here" —
  rather than silently shifting the marker one cell up the screen, and the status line
  says "moved the goal here" when it does move.
* **The eraser always erases.** `remove_goal_at` removes the last flag without
  complaint, so a document can have none. The three things that care each say so:
  `validate`/`to_json_pretty` refuse it ("no goal"), `reach::report` answers "the level
  has no goal flag" instead of indexing an empty goal list — which is what it used to
  do, a panic inside the editor's per-frame oracle run, on the frame after the flag was
  erased — and the info panel reads "no goal — place one before saving".
* **The loader holds the same rule as the editor.** A file with two goal entities, or
  a `G` tile beside one, is refused with "a level has exactly one flag"; `validate`
  refuses a second goal pushed into `goals` directly, so the writer cannot emit one
  either.
* `ResizeReport` loses `dropped_goals`. With one flag there is nothing to drop: a
  shrink drags the flag back in rather than losing it, which is what `goal_moved` was
  always reporting for the last of the pack.

One guard rail comes with the new state: pressing `E` on a document with no flag is refused
with "no goal on this level: place the flag before training on it" rather than starting a
run whose agents have nothing to reach — a flag-less level scores zero progress for every
body and the panel would read "0 tiles to go" for all of them. The message lands in the
status bar of the editor the viewer is still in.

Held by two new `sim` tests (`the_goal_can_be_erased_away_and_there_is_never_more_than_one`,
`placing_a_goal_moves_the_one_that_is_already_there`), three in the app
(`the_goal_can_be_erased_away_and_the_editor_says_what_that_costs`,
`placing_a_goal_elsewhere_moves_the_one_that_is_there`,
`the_ghost_and_the_document_agree_that_the_goal_is_already_here`), and two cases added
under `sim::tests::levels::malformed_levels_are_refused`. The pack itself is asserted to
carry exactly one flag per level.

### 1.8 There was no leftward sprint, and the oracle agreed with the sim about it — **bug, reported from play**

> "Our agent still only seems to want to move right."

It did, and the reason was mechanical rather than statistical. The action table is
*(direction × verb)*, and it had a hole of exactly the kind 0.2.0 closed for jumping:
`Sprint` and `SprintJump` were **rightward only**. So on a level whose flag lies to the
left — a level a designer can build in the editor in a minute — the population had one
fast gear and it pointed the wrong way, and `shift` + `left` at the keyboard produced a
plain `Retreat`, so the player could not ask for the move either. The reachability oracle
had the matching hole: `trajectory_action` chose its action by *speed* alone, so a
leftward take-off was launched at `-sprint_speed` and then held an action whose horizontal
input is `+1`. It measured the model decelerating the very move it was testing, and
reported the result as a fact about the level: the **mirror of `l8-long-jump`** — the level
that exists to teach sprint — came back *unbeatable in principle*, with the culprit named:
`left short-jump → None`.

Three changes, one per layer, and the last is what makes the claim testable:

* **The table has no preferred direction.** `RetreatSprint` and `RetreatSprintJump` —
  indices 9 and 10, **appended** rather than slotted beside `Retreat`, because an action
  index is part of what a recorded genome and a fitness trace mean; every earlier index
  keeps its number. `the_two_moving_directions_offer_the_same_gears` asserts the two
  directions offer the same gears *by comparing them*, so a future gear has to be added
  twice, and `a_sprint_covers_more_ground_than_a_run_in_both_directions` asserts the pair
  is two speeds rather than two names for one action.
* **The oracle picks by direction *and* speed**, and the sim's `wants_sprint` is a gear of
  a direction rather than of the rightward pair.
* **The keyboard follows for free.** The window and the terminal playtest share
  `render::play::action_for`, so shift-plus-arrow is now that direction's fast move in both,
  and the control hint says `shift sprint` rather than implying a direction.

**Measured** (`train-cli train --stop-on-win`, first-win generation, one column per seed):

| level | as shipped | its mirror |
|---|---|---|
| `l2-first-gap` (pop 64, seeds 1-3) | 10, 2, 3 | **0, 2, 2** |
| `l5-tightrope` (pop 200, seeds 1-4) | 7, 14, 12, 6 | 20, 1, 8, 13 |
| `l8-long-jump` (pop 200, seeds 1-4) | 4, 3, 7, 11 | 18, 18, 17, 16 |

Every one of the nine shipped levels' mirrors passes the oracle — before this fix the
mirrored `l8` did not pass it at all, which is the difference between "hard" and
"impossible". On the two levels whose only obstacle is a pit the mirror is learned in the
same handful of generations (`l2` the same or better, `l5` overlapping at 6-14 against
1-20). The `l8` mirror is consistently about three times slower, and finding 2.9 is why:
with its walker removed the pair is 1, 1, 2, 1 against 5, 0, 5, 3 — and a walker patrols
toward the body on one side and away from it on the other, which is the difference the
numbers are measuring rather than a capability.

Held by `sim::physics::tests::the_two_moving_directions_offer_the_same_gears`,
`sim::physics::tests::a_sprint_covers_more_ground_than_a_run_in_both_directions`,
`render::play::tests::… (shift refines either direction)`, and — the durable one — the two
new symmetry tests that mirror the *content*:
`sim::tests::levels::every_shipped_levels_mirror_is_reachable_in_principle` and
`evo::tests::difficulty::a_mirrored_level_is_learned_within_the_same_budget_as_the_level_it_mirrors`.
They lean on a new `Level::mirrored_horizontal()`, which mirrors tiles, spawn, flag,
walkers and scenery through the same mutators the editor uses.

### 1.9 The gap probe looked wherever the body was already heading — **bug (learning)**

The observation had one more directional assumption, and it was the subtlest of the three
that have been removed so far. `gap_ahead` was measured in the direction of
`PlayerState::facing()` — `sign(vx)`, with `+1` when the body was standing still — so the
input described the body's **last decision** rather than the world, and a body at rest (the
state in which an episode's first, direction-choosing decision is made) was told about the
ground to its right whatever lay behind it.

It is now a pair: `gap_right` (index 83, byte-identical to what a rightward body already
read) and `gap_left` (a new index 94), one slot per direction, neither of them "forward".
`PlayerState::facing()` is gone — nothing else wanted it — and `sensors::VERSION` moved to
**v4**, which is the trio's job: the layout moved, so every old champion is refused rather
than replayed against an observation whose index 83 means something new. The golden
observation hash is re-pinned, deliberately and in the one place that is allowed to move it.

Held by `sim::tests::sensors::the_gap_slots_fire_on_the_side_the_ground_runs_out` (a
stationary body on a ledge, and the same body on the mirrored ledge: the two slots swap)
and `the_gap_slots_ignore_the_direction_the_body_is_moving` (standing, running right and
running left all read the same), plus the relayouted
`the_vector_has_the_documented_named_layout` and the re-pinned
golden hash.

---

## 2. Bugs and rough edges found, not yet fixed (prioritised)

### 2.1 The spawn point can be placed inside solid terrain — **medium**

`Editor::can_place` special-cases only `Walker` and `Scenery`; `Brush::Spawn` calls
`Level::set_spawn_tile`, which clamps into the grid but does not require the target
cell to be **empty**, and `Level::validate` checks only that the spawn is in bounds.
A level whose spawn overlaps a wall is one the agent dies in on frame 0, and the
editor says nothing beyond the oracle's verdict.

**Fix:** refuse a spawn on a non-empty cell and say why, exactly as a walker with no
ground under it is refused (the pattern and its message already exist). Optionally
auto-clear the cell.

### 2.2 A goal flag needs no ground and may be buried — **low**

`Level::goal_placeable` now refuses an off-map tile and a marker that overlaps the one
already there (1.7), but nothing requires the goal to **stand** on something: a flag floating in
the air, or inside a wall, is a level the oracle will (correctly) call unbeatable while
the editor lets you place it without a word. Same fix shape as 2.1: a goal is a thing
that *stands* (and its two cells should be checked, which is 1.6's rule). A related
nit this pass left alone: a goal anchored on the **top row** draws its flag one cell
above the map, so the marker is a bare pole up there.

### 2.3 Hashed dressing and placed scenery can share a cell — **low, cosmetic**

`view::scenery_sprite` scatters by tile hash and the new placed pass draws on top, so
a decorated tile can show two props. Harmless, but the editor could suppress the
hashed prop wherever a placed one exists (a one-line test in the scatter pass), which
would also make placement feel precise.

### 2.4 The nearest-enemy sensor sees through the whole level — **medium (learning)**

Finding 17, still true: `sensors::observe` gives slots 81–82 the nearest live
walker's `dx/dy` clamped at `ENEMY_VIEW_DISTANCE = 4.0` tiles, so a walker 80 tiles
away reads exactly like one 4 tiles away, and any level with a walker hands every
agent a constant "enemy that way" input from tick 0. An off-grid walker (now refused
by the pack test) did the same. The fix is a "no enemy in sight" state, i.e.
`sensors::VERSION` 3 — a deliberate, vision-improving change that FINDINGS.md already
argues for.

### 2.5 `--trials` is off by default, so walker levels are noisy — **medium (learning)**

Finding 14 established that `episode_seed` mixes in the generation, so on the one
stochastic level an elite re-rolls the walker's patrol phase every generation and can
lose the run that won. `--trials` averages that noise but defaults to 1. A default of
2–3 trials *only on levels containing walkers* would stabilise selection where it
matters without paying for it on the flat levels. Cheap, no version bump.

### 2.6 No automated check runs the determinism suite

The determinism contract is the spine of the project and there is no `.github/`
workflow (or equivalent) to run `cargo test --workspace` on a change. The tests are
excellent; nothing enforces them.

### 2.7 `crates/app/src/view.rs` is ~6.1k lines

One module holds the camera, terrain autotiling, the painted backdrop, the prop
scatter, the ambient layer, agent animation, the replay field and the frame harness.
It is well-commented, but it is the single most likely place for the next
"looks fine, is wrong" bug to hide. Splitting it (backdrop / terrain / props /
agents) is a mechanical, test-covered refactor.

### 2.8 Save can silently overwrite a shipped level — **medium, and it already happened once**

`App::save` gives an *untitled* document a filename derived from its own name inside the
level directory (`levels_dir().join(slug(&level.name))`). A document that is named after a
level the repository ships therefore writes over that level, with no confirmation and no
backup — and a document can pick up such a name without anyone typing it, because opening
a level sets `editor.path` (so a save goes back to the same path, which is right) and the
name travels with the doc from there.

This is not hypothetical: `levels/l1-flat-run.json` was found in this pass holding a 10×20
scramble — not the 60×15 runway the pack, the difficulty ladder and the render tests all
describe. Nine app tests, both `render` playability tests, a `train-cli` pack test and an
`evo` difficulty test were failing because of it, and every one of them passed again after
`git checkout` restored the file, which is how the cause was identified rather than guessed.
The failure mode is worth naming: the damage is invisible in the suite that *tests* the
editor and only shows up as mysterious failures in tests about *content*.

A fix would be a confirmation when the target exists — or, cheaper and better, writing new
documents under a `scratch-` prefix and never to a name the pack already uses. Neither is
done here, because the level it clobbered was restored and no code path in the current suite
writes into `levels/` (verified: the file's hash is unchanged across a full `cargo test
--workspace`).

### 2.9 A walker has a position but no heading, so it always sets off rightward — **low, and the last direction in the level format**

`Walker::new` builds every walker facing right, and the level file records only where a
walker stands: a patrol's first direction is a constant. On a rightward level that reads as
a harmless default. On a leftward one it is the one thing that does not convert — a body
moving left meets a walker moving *toward* it, where the mirrored geometry had it moving
away, an ambush rather than a patrol, on the pad between two pits, which is exactly where
`l8-long-jump` puts one.

Measured, which is why this is a finding rather than a hunch: `l8-long-jump` and its mirror
are learned at first-win generations 4 / 3 / 7 / 11 against 18 / 18 / 17 / 16 (pop 200,
seeds 1-4); delete the walker from both files and the same pair is 1 / 1 / 2 / 1 against
5 / 0 / 5 / 3. The three-fold, six-of-six-consistent gap is the walker. The same test on
`l5-tightrope`, which has no walkers, is symmetric to within noise (7 / 14 / 12 / 6 against
20 / 1 / 8 / 13).

A fix means a `heading` (or `facing`) field on the walker entity — a level-format change
plus an editor control the editor has no walker-direction brush to hang on yet. Worth doing
when walkers get their own pass; [`Level::mirrored_horizontal`] documents the limitation
until then, so a symmetry measurement on a walker level is read for what it is.

### 2.10 A checkpoint is only credited for a crossing on the level's *left* end — **low (reward)**

`Level::landing_zone` + `Level::zone_is_checkpoint` decide whether arriving on a stretch of
ground meant crossing something by looking at the tile to the left of the zone's start. On
a rightward level that is exactly the right question. On a leftward one the crossing is at
the zone's *right* end, so a landing that should pay can pay nothing — the one remaining
rightward assumption in the *reward* (the observation's last one went in 1.9, and the cap
in 1.8).

Left unfixed deliberately, and small: the term is worth [`CHECKPOINT_BONUS`] = one tile (`16`)
per zone against a goal worth `10 000`, and on the shipped pack the mirror of each level
collects the same count as the level (2 and 4 on `l5` and `l8` respectively). The obvious
symmetric-looking fix — "credit a zone if *either* end is a crossing" — is **wrong** and is
worth stating so nobody reaches for it: it would pay a body for standing on the near side
of a pit it has not crossed yet, which is the one thing the term exists not to do (its own
docstring: a checkpoint is for a crossing, and a leap that ends in the air keeps its
frontier and nothing else). A correct fix needs the *arrival side*, which the sim does not
currently hand to `checkpoint_zone` — a signature change plus a test per direction, for at
most a few dozen points on a level that has that many zones.

### 2.11 The Open menu lists raw filenames and there is no starter pack

The README already says M4 is missing "the curated starter pack and a browsable level
shelf". A new player sees `l1-flat-run.json … l7-wall-school.json` and a
`scratch-l1-walkers.json` with no "start here".

---

## 3. Feature improvements (existing features)

1. **A draggable rectangle/fill tool.** Painting is brush-only; laying a platform is
   a stroke. A box-select/fill (and a 1-tile "platform" stroke) is the single biggest
   authoring speed-up, and it is contained (`Editor` already has `begin_stroke`/undo).
2. **A spawn/goal "standable" rule** (2.1, 2.2) plus a status-line hint, matching the
   walker rule that already exists.
3. **A "playtest from here"** start, so a designer can test the obstacle they are
   looking at rather than the whole level from the spawn.
4. **Reachable-without-caveats surfaced in the palette.** The oracle already reports
   "beatable over N platforms"; a per-obstacle readout (the `culprit` field exists)
   could name the first jump that fails while you draw, next to the cursor.
5. **A level shelf / starter pack** (2.11) with thumbnails rendered by the existing
   `--screenshot` path (a 1-frame render per level is cheap and already hardware-light).
6. **A "menu" key** to return to the title screen; today the splash is reachable only
   by restarting the app.
7. **Save/load presets for scenery** — the picker remembers the last sprite (it does),
   but there is no "scenery set" a user can name and reuse.

---

## 4. AI and learning improvements

The learning loop is already strong: fixed-topology GA, edge-triggered jumps with a
press buffer, a per-jump cost, a reachability oracle split from the difficulty
oracle, and a stall→checkpoint→curriculum escape that is *measured*. The two research
passes and the repo's own findings agree on where the remaining gains are, and they
all point at the **fitness landscape**, not the search.

The headline problem is finding 15, the **park trap**: `+1` per new rightmost pixel
plus `+0.05`/frame survived against `-120` on death means that once a jump is
clearable only from a narrow window, parking short of it out-scores dying on it, so
selection *removes* whatever tried. A maximal pit before a spike patch was never
solved in 420 generations, while the same level without the patch fell at generation
36. This is a reward-design problem, and reward design is the researched fix:

1. **Re-shape the score around progress and checkpoints, keep survival as a
   tie-break.** Remove or cap the per-frame survival term and the per-jump cost; make
   success dominate everything (it already does at +10 000). Credit *maximum* progress
   so a failed attempt still scores for the frontier it reached. Intermediate
   checkpoints ("a safe landing zone beyond an obstacle") are the concrete fix for the
   pit that gives no feedback until the jump succeeds. This is exactly the maze/cul-de-sac
   failure documented for novelty search ([Woolley & Stanley](https://arxiv.org/html/1207.6682v1))
   and the potential-based-shaping form `F = γΦ(s') − Φ(s)` that preserves the optimal
   policy under its assumptions ([PBRS](https://arxiv.org/html/2502.01307v1),
   [Ng et al.](https://www.cs.utexas.edu/~shivaram/readings/b2hd-NgHR1999.html)).
   *Cost:* every term is versioned (`fitness::FITNESS_VERSION`, and the trio), so this
   invalidates champions and must be re-measured against
   `crates/evo/tests/difficulty.rs` and the FINDINGS.md difficulty table.

2. **Keep the one good negative term, and make its intent explicit.** The per-jump
   cost breaks a real tie and is 8× below the value of the cheapest useful jump. It is
   the *only* negative term that is defensible; the survival bonus is the one paying
   for "stand still and live". Prefer removing survival to adding more penalties.

3. **Quality-diversity to escape the plateau.** The stall→curriculum escape (finding
   22) works but is a *policy* fix applied once. A coarse **MAP-Elites** archive keyed
   on two behaviour descriptors — furthest obstacle crossed, furthest safe landing
   region — would keep the "back up, then jump" lineages alive instead of deleting
   them, which is precisely what the park trap deletes
   ([Mouret & Clune](https://arxiv.org/abs/1504.04909),
   [ME-ES](https://arxiv.org/html/2003.01825v3)). This fits the existing verbatim
   `Packet`/`Trainer` boundary: the archive rides beside the population and only
   changes which parents are sampled.

4. **Diversity-triggered mutation, not time-triggered.** Finding 20 measured that a
   *mid-run* mutation-rate change does not break a stall, because the landscape, not
   the step size, is the trap. The literature's version is condition-triggered: raise
   the large-mutation share or inject immigrants only when improvement **and**
   behavioural diversity are both flat ([Katoch et al.](https://pmc.ncbi.nlm.nih.gov/articles/PMC7599983/)),
   and consider evolving a per-genome `log σ` ([Ye et al.](https://arxiv.org/html/2303.04611v2)).
   The stall detector already exists — this would hook it to the *breeder*, with the
   archive as the diversity signal.

5. **Action-space refinements worth measuring, one at a time.** Edge-triggered jumps
   already landed (v2). The remaining candidate is **action repeat as a decision**: a
   small set of durations (1/2/4) or early interruption on a landing/collision, which
   is what the dynamic-action-repetition literature reports helps on harder tasks
   ([Lakshminarayanan et al.](https://ojs.aaai.org/index.php/AAAI/article/view/10918)).
   The 4-tick fixed repeat is a reasonable current compromise; treat this as an
   experiment, not a conclusion.

6. **The sensor fixes** (2.4 "no enemy in sight", 2.5 default trials) are the cheap,
   no-landscape-change wins and should be done first.

7. **NEAT stays deferred.** The plan gates it on an observed plateau; the fixed
   93→16→9 net is small enough that reward/behaviour work pays first
   ([NEAT-Python research notes](https://neat-python.readthedocs.io/en/latest/academic_research.html)).
   CMA-ES/ES remain useful *baselines* to compare against the GA on equal episode
   budgets ([Hansen](https://arxiv.org/html/1604.00772),
   [Loshchilov](https://arxiv.org/html/1404.5520),
   [Salimans et al.](https://arxiv.org/abs/1703.03864)), not upgrades.

**Recommended order:** 2.5 and 2.4 (cheap, no version bump) → fitness reshaping +
checkpoints (version bump, re-measure) → MAP-Elites archive → diversity-triggered
mutation.

### Delivered since this was written

Item 1 (partial — the survival half) and item 2 landed as **fitness v3**, and the
action-space question behind item 5 turned out to have two concrete defects in it. All
three are measured in FINDINGS.md findings 46–48; the short version:

* **The jump ladder had a dead rung.** `min_jump_hold` was two decision blocks, so the
  first release the agent could ask for was ignored and holding for one block and two
  blocks gave the same jump — ladder `{1.70, 1.70, …}`. Now one block, ladder
  `{1.155, 1.70, 2.085, 2.31, 2.375}`.
* **`brake` was inert in the air** — bit-identical to `walk`, because the friction branch
  only reached for `brake_friction` when grounded. With it biting mid-flight, hop *carry*
  goes from a minimum of 2.83 tiles to a continuum down to 0.90, which is what makes a
  floor of hazards hoppable rather than merely survivable.
* **The per-frame survival term is gone** and a death costs 1.25 tiles instead of 7.5, so
  attempting a hazard pays above roughly a one-in-four success rate instead of 79%.

Measured on the level shape that produced the owner's report (one-tile spikes, one safe
tile between them): the best of the population parked at `x = 7.4` — the last tile before
the first spike — and the level went unsolved through **200 000 episodes** (pop 500 × 400
generations, three seeds) before the pass; after it the same level is **solved at
generation 110 at pop 1000**, with its two-safe-tile sibling at 26–43 generations at pop
500. The `7.4` figure is from the `evo::fitness` module docs, which is where the parking
measurement was first taken.

Item 1's **checkpoint** half is the one that did the most work, and it landed as
**fitness v4** (FINDINGS.md finding 49). A checkpoint is a safe landing zone the body *stood on*
past a hazard or a hole, paid once; the counting lives in the sim, and the definition is
*stricter* than the frontier rather than a second copy of it, because the frontier is credited
for the airspace over a pit the body then falls into and a checkpoint is not. On the tight
hazard floor this moved the population's plateau from **one hazard crossed in 400
generations** to **thirteen of fifteen**, and took a level that needed pop 1000 to fall down
to a solve at pop 200 (generation 107). On the easier hazard floor it is a wash — the
gradient was already there — and on hazard-free levels it is inert by construction:
`l1-flat-run` cannot collect a single point of it. The shipped difficulty ladder passes
unchanged.

What this *does not* settle, and what still stands from the list above: item 1's
**checkpoints** half is untouched (nothing yet credits "a safe landing zone beyond an
obstacle"), the MAP-Elites archive and diversity-triggered mutation are still proposals,
and 2.4/2.5 are still open. The one measured non-finding is worth recording so it is not
re-tried: **the decision cadence is not the bottleneck.** `--action-repeat 3` (a finer
control granularity than the default 4) reached `best_x 25.4` on the tight floor but
finished *no* seed inside 800 generations at pop 1000,while the default cadence finished one — so the cadence stays 4.

### Item 4, done and measured: the breed no longer only jumps

Item 4 above asked for condition-triggered *immigrants*, hooked to the stall detector.
That landed, together with the second mutation scale the search was missing, and the
verdict is narrower than the item hoped. A textbook GA makes **one** move — a coarse
Gaussian step over five percent of the genome, so a child is about eighty-six genes away
from its parents, a jump rather than a step — and it only ever makes it at a size the
operator chose before the run started. Three things now sit beside that in
`evo::ga::Trainer::breed`:

* **Fine children** (`--fine-children`, default 8). Eight offspring a generation are
  perturbations of *one to three genes* of the generation's best genome. A coarse child of
a nearly-good lineage is a fresh lottery ticket; this is a step.
* **A reheated restart on a plateau** (`--stagnation`, default 20; `--immigrants`, default 8).
  After twenty generations with no new best, eight copies of the run's **best-ever genome** are
  bred in *beside* the elites — each with **every gene** moved by the reheat step, which is
  **three times the run's own mutation size** (0.90 at the shipped sigma), so the batch lands far
  from the anchor while still being pulled towards it — and then the counter restarts so the run
  gets a full interval to use them. The first version of this bred in eight *random* genomes instead, which throws away
  everything the run has learned; the numbers for both are below. The name `immigrants` stayed,
  because these genomes still arrive from outside the population's own offspring — only their
  source changed.
* **A self-tuning step** (`--adaptive-sigma`, **off by default**, see below). Rechenberg's
  1/5th rule, and the one addition that measured a loss.

The two mechanisms that are on cost nothing measurable on the levels this pack solves,
which is the first thing that had to be true: at pop 200, `--stop-on-win`, the same
level and seed, generations to the first win, **12 seeds** per arm, the textbook arm being
`--fine-children 0 --stagnation 0`:

| level | textbook | with both mechanisms |
|---|---|---|
| `l2-first-gap` | 12/12 solved, mean **2.1** (1 1 1 0 3 1 0 3 3 0 5 7) | 12/12, mean **2.2** (1 3 2 0 4 2 0 4 1 0 3 6) |
| `l5-tightrope` | 12/12, mean **14.3** (7 14 12 6 8 4 20 29 39 5 14 14) | 12/12, mean **15.8** (10 11 12 16 12 25 32 10 8 30 15 9) |
| `l8-long-jump` | 12/12, mean **9.3** (4 3 7 11 10 18 3 17 11 12 13 3) | 12/12, mean **9.0** (5 3 3 8 3 5 6 12 4 9 42 8) |

A six-seed run of the same comparison had looked like a 1.7x regression on `l5-tightrope`
(mean 8.5 against 14.3). Twelve seeds show both arms at 14-16 with the same spread, so
that was the lottery moving, not the mechanisms: with fine children on, every coarse child
consumes a different slice of the RNG stream, so *per-seed* pairs are not comparable at
all and only the distribution over many seeds is evidence. Two things follow, and they are
recorded rather than buried: the mechanisms are a wash on this pack's solvable levels, and
any future claim about them needs at least a dozen seeds.

**The hard level is where the restart earns its place, and the reheat beats the restart.**
`levels/scratch-tower.json` — the 10x22 zig-zag, a plateau at 128.3 that the pack's solvable
levels do not have — at pop 200, a 200-generation budget, **12 seeds**, generations to the
first win (`-` = never inside the budget):

| arm | solved | mean when it did | generations spent solving-or-giving-up |
|---|---|---|---|
| textbook (flat 0.30, no fine children, no restart) | 5/12 | 165.8 | 2229 |
| fine children + **random** immigrants | 5/12 | 65.8 | 1729 |
| fine children + **reheated** immigrants (shipped) | **6/12** | **78.2** | **1669** |

Per seed: textbook `167 172 161 - - - - - 132 - - 197`; random `- 37 - - 101 - 34 98 - 59 - -`;
reheated `- 37 67 - 62 - 34 - - - 169 100`. Two things the table says that a mean alone would
hide. The restart — either kind — is worth **2.5x** the textbook arm's generations when it
lands, because a plateau is the only thing that separates the arms at all. And the reheat is
not just a prettier random restart: it solves one more seed, spends less of the budget, and,
by the shape test in `genome.rs`, explores around the champion instead of discarding it.

The reheat also **beats the random restart it replaced** on the same twelve seeds: 9/12 and every
summary statistic, once the temperature is set as below. Nothing here claims the 6-against-5
difference at 1.0x is significant on its own; what it establishes is that the reheat costs nothing
on any statistic and has the better mechanism story.

**The temperature sweep, which is what set the default.** The reheat is thrown at
`mutation_sigma × reheat_factor`, held inside the documented 0.05-1.00 sigma band. All four arms,
`scratch-tower`, pop 200, 200-generation budget, 12 seeds:

| `--reheat-factor` | reheat sigma | solved | mean when solved | generations spent |
|---|---|---|---|---|
| 0.5 | 0.15 | 7/12 | 93.3 | 1653 |
| 1.0 (the first default) | 0.30 | 6/12 | 78.2 | 1669 |
| 2.0 | 0.60 | 7/12 | 103.9 | 1727 |
| **3.0 (shipped)** | **0.90** | **9/12** | 103.3 | **1530** |

0.5, 1.0 and 2.0 are within noise of each other, so the gentle end of the range does not matter;
the hot end does. Twelve seeds is too few to trust that on its own, so 1.0 against 3.0 was rerun
over **24 seeds**: **14/24 against 10/24**, paired seeds 5 gained for 1 lost, and less budget spent
(3628 against 3850). Per seed, 1.0x solved `2 3 5 7 11 12 13 14 16 23` and 3.0x solved
`2 3 5 6 7 8 10 11 12 13 14 17 21 23` — the extra five are 6, 8, 10, 17 and 21, and only seed 16
goes the other way, so mixing the two temperatures is not a third option (the union is 15/24,
against 3.0's 14 alone).

What the sweep does **not** establish, and this is the honest reading: the paired exact test on
those 24 seeds is p ~ 0.2, so the *size* of the gain is unproven, and when the hot batch does solve
it takes slightly longer (median 121 against 101) — it buys reliability, not speed. The direction is
consistent in both samples and with the mechanism's own theory (a local-optimum trap needs
displacement, and the 1.00 ceiling is what stops displacement becoming a restart), and no
measurement anywhere favours the cooler default, so 3.0 ships — with `--reheat-factor` available
for a run that wants the gentler batch, and with this paragraph as the reason a future reader
should not read more into 14-against-10 than it says.

**A defect the sweep found in the command line itself, which is the part worth remembering.** The
first version of `--reheat-factor` was written with its own default beside the flag name —
`a.f32_opt("reheat-factor", 1.0)` — while the trainer's own default is a field of `GaConfig`. When
the sweep moved that field to 3.0, every front-end that did not name a factor went on asking for
1.0, so a whole sweep's result would have shipped as a flag nobody's run would ever use. Nothing
tested it: the unit tests build `GaConfig` directly and never touch the flag, and the run header
did print the number — it printed `reheated at 1.00x (0.30)`, which read as correct because it was
the number the run was using. It was caught by reading the header of a real run after the change,
which is the only place the two defaults meet. Fixed by giving the flag no default of its own (it
reads `GaConfig::default()`, refactored into `train_config` so both front-ends do) and guarded by
`the_command_lines_defaults_are_the_trainers_defaults`, which compares the config a bare `train`
invocation produces against `GaConfig::default()` field by field — and which was verified to fail
when the old default is put back (`left: 1.0, right: 3.0`) rather than assumed to work.

One consequence is worth stating plainly, because the name could mislead: at 3.0 the batch is thrown
0.90 at genes whose own scale is about 0.15, so most of each child is noise and the champion
survives only as a **bias** — measured, the batch agrees with the anchor on 0.565 of its genes
against the 0.50 a random genome manages, and the gene correlation is above 0.05 where theory puts
it near 0.16. It is a biased restart, not a local step, and that is exactly what it is for.

**The change is provably inert on a level without a plateau, which is the cleanest check in
this pass.** A burst can only fire after twenty still generations, so on the levels the pack
solves in under twenty the reheat should never run — and because it therefore never consumes a
draw from the breeding RNG, the same seed must produce the *same run*, not a similar one. It
does, exactly: `l2-first-gap` and `l5-tightrope` are **bit-identical, 24 of 24 seeds**, mean
2.2 and 15.8 generations before and after. `l8-long-jump` differs on precisely one seed out of
twelve — seed 11, 42 generations → **29** — which is a run that did plateau and did get
reheated (mean 9.0 → 7.9). A prediction that could have failed, made before the change, and
it is the reason the tower table above can be read as the restart's doing rather than as a
reshuffled lottery.

**Rechenberg's rule measured a loss, so it is off by default.** The rule (`--adaptive-sigma`
moves `mutation_sigma` between 0.05 and 1.00) was implemented three ways and every reading
lost on the tower, on seeds 1-3 against the textbook arm's 167/172/161:

| reading of the rule | tower seeds 1-3 | why it fails |
|---|---|---|
| shrink on a low success rate | never solved, never solved, never solved | 0.30 → **0.05 within eight stagnant generations**; the children become six-thousandth-of-a-weight twitches and the lineage freezes on the plateau |
| grow on a plateau instead | never solved, **126**, never solved | spends the run at the **1.0** ceiling and breaks the jump timing the level needs |
| hold the size on a plateau | never solved, never solved, never solved | still bleeds 0.82 per generation in the stretches where a best *is* found, and ended at 0.050 and 0.061 on the seeds it failed |

The first reading is the classic one and the diagnosis is worth keeping: a sub-20 % success
rate means "the step is overshooting" on a slope and "the population cannot leave this
value" on a plateau, and the rule cannot tell them apart. This is finding 20's conclusion
arriving from the other side — the landscape, not the step size, is the trap — and it is
why `adaptive_sigma` is a flag rather than the default. What is *kept* is what the same
measurements support: the run is bred at a constant step, and a plateau is answered by the
two mechanisms that can answer it, a local step and a restart.

---

## 5. Graphics and assets

The art pipeline is exceptional — a generator, a manifest, palette shared by one
module, and tests that read the shipped PNG — and the last few findings (32–35) fixed
the real readability problems (player camouflage, invisible props, floating tree-line).
What remains is mostly *motion* and *feedback*:

1. **Juice is missing.** There is no audio at all, no screen shake, and no hit-stop.
   The researched ordering is: make controls responsive (already good — coyote time,
   jump buffer, corner behaviour are all in the physics bible), then add feedback per
   action — takeoff anticipation, landing dust (exists), a stronger impact for a stomp
   or death, a sparkle for the flag, and a short hit-stop on death/stomp only. Screen
   shake needs a defined intensity ladder and a meaning per event
   ([GameAnalytics juice](https://www.gameanalytics.com/blog/squeezing-more-juice-out-of-your-game-design),
   [Feel: screen shakes](https://feel-docs.moremountains.com/screen-shakes.html)).
2. **Camera legibility.** The follow camera pins the floor and tracks x (good). The
   remaining 2D-platformer polish is a focus zone and a look-ahead in the direction of
   travel, plus a "panic" follow near the frame edge
   ([Game Developer: camera logic](https://www.gamedeveloper.com/design/camera-logic-in-a-2d-platformer)).
3. **Transitions.** Editor→play→evolve→replay are hard cuts. A short, purposeful
   wipe/fade (and an immediate, skippable return) is what the intro research calls a
   hand-off rather than dead time
   ([GameAnalytics intros](https://www.gameanalytics.com/blog/create-immersive-game-intros)).
4. **Readability checklist as a test, not an eye.** The art suite already asserts
   structural properties; extend the same idea to a *contrast* rule per surface
   material the way finding 32 did for the player, so a future material cannot ship
   camouflage ([Aran Ink](https://aran.ink/posts/celeste-tilesets),
   [SLYNYRD](https://www.slynyrd.com/blog/2020/5/21/pixelblog-28-side-view-tiles)).
5. **Achievability constraint:** the atlas is 149 of 160 cells. Any *new terrain
   material* (three families × masks + blends) needs the grid widened first; the
   generator supports it (`COLS`/`ROWS` + the manifest), and the art tests already
   refuse an atlas that overflows. That is why this pass added placeable props (which
   reuse existing cells) rather than a new material — see §6.

---

## 6. Asset variety — what landed, and the roadmap

**Landed:** a scenery layer with 17 curated placeable props (bushes, tufts, three
flowers, rock, pebble, two mushrooms, log, stump, reeds, vine, two trees), a palette
picker, size/animation/canopy handled by the existing view code, and full file
round-trip. See §1.4.

**Next, in order of value per unit of risk:**

1. **More scenery in the picker** (near-free): butterflies are ambient, but a placed
   `pine_*` silhouette, the second reed/vine frame, and `mushroom_pair` variants cost
   one list entry each.
2. **New terrain materials** (needs atlas widening + `sim` `Tile` variants + art, the
   sand pattern): a **stone brick** and **wood plank** material would double the
   authored vocabulary of a level. No `SIM_VERSION` bump is needed for a *purely
   visual* material (sand set the precedent: sensors report it as solid), but the
   atlas must grow first.
3. **Gameplay entities** — a **spring/bounce pad** and a **moving platform** — are the
   real "fun" additions, and the real cost: they change physics and reachability, so
   they are a `SIM_VERSION` + `sensors` + `fitness` trio bump and an oracle update.
   Worth doing deliberately, with the difficulty table re-measured.
4. **A hazards family** — ceiling spikes and a saw/slime — meaningfully changes level
   design and is a smaller physics change than 3.

---

## 7. Splash screen

Done this pass (§1.5): the named keys now enter the activity they name, the invitation
is clearer and reads over any world, and the band fades in with the words.

Remaining polish, per the intro research: make the three activities into
**clickable buttons with hover/pressed states** (the screen is currently key-driven
only), show a **third option once a run exists** ("watch the run"), and add a short
confirmation on the first click. The research also recommends distinct, readable
control states and, for accessibility, remembered settings — the footer already
carries the version and the "edit · play · evolve · replay" reminder
([GameAnalytics intros](https://www.gameanalytics.com/blog/create-immersive-game-intros),
[Game Accessibility Guidelines](https://gameaccessibilityguidelines.com/full-list/)).

---

## 8. Verification

* `cargo test --workspace` — **413 passed, 0 failed** (403 before this pass, +10:
  scenery round-trip, splash action, splashed-frame scenery, editor scenery brush;
  the splash dismissal test was updated for the new return type; then +6 for finding
  1.6 — a goal addressed by its flag, a tree by its canopy, a flag that cannot be
  stacked, the eraser taking a flag by the cloth, the ghost agreeing with the
  document, and the frame/cell correspondence).
* `cargo clippy --workspace --all-targets` — **clean**, no warnings (the one
  pre-existing lint was in the new `splash_frame` and is fixed).
* Rendered and diffed real frames from the app: a decorated L1 differs from a bare L1
  by 9 372 pixels with placed prop colours present (e.g. `MUSHROOM_CAP`), the new  title screen renders the invitation band and both named keys, and a demo level with a
  flag renders (and opens back in the editor) with the marker drawn two cells tall.
  That demo carried a **second** flag at the time, chosen to show both markers; finding
  1.7 makes a two-flag file unloadable, so that scratch file is gone and the frame
  stands as the marker's own render. Frames: `out/scenery-demo.png`, `out/l1-bare.png`,
  `out/splash-new.png`, `out/goal-demo.png`, `out/goal-zoom.png`, `out/fix-l1.png`.
* Finding 1.7, later in the same pass: `cargo test --workspace` — **428 passed, 0
  failed** (two for this rule, and one render fixture rewritten) — with the loader
  refusal reproduced through the real interface rather than
  only in a unit test: `train-cli reach` on a hand-made two-goal file exits 1 with
  `level has more than one goal: a level has exactly one flag`, and on a `G` tile
  beside a goal entity with the "`'G'` tile and a goal entity" wording.
* Findings 1.8-1.9 (the leftward pass, sensor v4): `cargo test --workspace` — **432
  passed, 0 failed**; `cargo clippy --workspace --all-targets` — clean. The two
  failures this change did cause were both in tests it had just written:
  `from_index`'s out-of-range clamp now lands on the table's last row rather than on
  `Walk`, and the mirrored-sprint test spawned its body at the starter level's left
  edge, where the leftward leg measured the wall instead of the gear.
* **A pre-existing failure that was not mine, and the reason it looked like mine.** The
  first full run of this pass reported 9 app, 2 render, 1 `evo` and 1 `train-cli`
  failure. All thirteen were one clobbered file (`levels/l1-flat-run.json`, finding
  2.8), not the sensor or action changes: restoring it from HEAD turned every one of
  them green without another edit. Checked rather than assumed — the same suite at
  `HEAD` in a scratch worktree passes, and the file's hash is identical before and
  after a full workspace run, so the *test suite* is not what writes it.
* Leftward learning, measured through the real trainer rather than asserted. Every
  shipped level's mirror is oracle-beatable, and the pairs are: `l2-first-gap` 10 / 2 /
  3 against its mirror's 0 / 2 / 2 (pop 64, seeds 1-3); `l5-tightrope` 7 / 14 / 12 / 6
  against 20 / 1 / 8 / 13; `l8-long-jump` 4 / 3 / 7 / 11 against 18 / 18 / 17 / 16 (pop
  200, seeds 1-4, `--stop-on-win`). The `l8` gap is its walker, not a direction: with the
  walker removed the same pair is 1 / 1 / 2 / 1 against 5 / 0 / 5 / 3 (finding 2.9).
* **The tower is now symmetric, and it is a lottery either way.** The vertical level the
  objective pass was built around (`levels/scratch-tower.json`, the 10x22 zig-zag) and
  its mirror, at pop 300 over a 400-generation budget on seeds 1-3: **solved at 327 /
  275 / — against 289 / 271 / —**. Two of three seeds in each direction, generations
  within a few percent of each other, and the same seed (3) failing in both — which is
  the cleanest evidence available that the leftward capability gap is closed rather than
  shifted. (Before the sensor change the same level solved on none of the three seeds,
  while its mirror solved on two of three: the split gap probe lands a `gap_left`
  exactly where a rising zig-zag needs one.) It remains a lottery at this budget in
  either direction, which is why the test suite pins the *reachability* and the mirror's
  learnability on `l2` rather than on this.

* **This pass (the search: fine children, a reheated restart, an optional self-tuning
  step).** `cargo test --workspace` — **444 passed, 0 failed** (the last pass ended at 437;
  this one adds the reheat operator's own test, the panel line's, the temperature mapping's,
  and the measured-temperature and hot-end-anchoring tests, and rewrites the immigrant test
  rather than adding one. The new coverage: the fine children are
  local steps, a stagnant run keeps its step instead of shrinking it to the floor, the
  rule's own arithmetic in both directions plus its sample guard and its clamp, migrants
  land only on the stagnation threshold and are reheats rather than restarts, the search's
  bookkeeping is as reproducible as the rest of the run, and a reheat moves every gene while
  keeping the genome's shape; the app's panel line gained one). `cargo clippy --workspace
  --all-targets` — clean. Through the real CLI, not only the unit tests: the training table
  carries a `sigma` column and the run summary a `sigma 0.30 -> 0.300 … reheated 0 in 0
  bursts` line, the CSV gained `mutation_sigma` and `immigrants` columns, and a champion
  written by this build records all four new knobs and still replays with **verified
  identical trajectory**. The A/B against the textbook arm is in §4.
* **What the A/B cost, and how it was read.** 4 levels x 6 seeds x 2 arms at pop 200, then
  3 levels x 12 seeds to check a suspected regression that turned out to be the lottery (§4),
  then 5 further arms on the tower alone to attribute it, then 12-seed arms for the reheat
  against both the textbook arm and the random-immigrant one, then a 4-arm temperature sweep
  at 12 seeds and a 2-arm confirmation at 24: about 320 training runs, every one of them
  through `train-cli train`, none over four seconds. The wall clock per generation is
  unchanged by the mechanisms — **0.003-0.021 s/gen** at pop 200 across all of them, i.e.
  ~10 000 population-episodes per second on this machine — so nothing here traded speed for
  cleverness.
* **The temperature sweep is also a safety check on the levels that do not plateau.**
  `l2-first-gap`, `l5-tightrope` and `l8-long-jump` at 12 seeds each, factor 1.0 against the
  shipped 3.0: **36 of 36 identical solve generations** (means 2.2, 15.8 and 7.9). The hotter
  batch costs nothing where a plateau never happens — and where one does (l8's seed 11) the
  *outcome* is the same 29 generations, which is stated as a fact about the solve rather than
  about the trajectories: once a batch fires, the genome values differ even though the number
  of RNG draws does not, and what is observed is that the winning lineage is unaffected.

**Not verified:** the pointer feel of finding 1.6's ghost, and any interactive
clicking at all — there is no display in this environment, so every app check goes
through `--screenshot` and the logic-level tests. The tile the mouse reports is
`Camera::to_tile`'s, which `the_flag_is_drawn_in_the_cell_the_eraser_addresses` now
exercises against a real frame, but a human at the window is still the only thing that
can confirm the drag reads right.

## 9. How the additions are held in place

| Claim | Test |
|---|---|
| Scenery survives a save/load and stays in the grid | `sim::level::editing_tests::scenery_round_trips_stays_inside_the_grid_and_is_not_hashed` |
| Decorating a level does not change its identity | same test (hash equals a bare starter) |
| A placed prop reaches the frame, a tree gets a canopy | `app::view::tests::placed_scenery_is_drawn_in_the_frame` |
| The scenery brush places its sprite; the eraser takes it first | `app::editor::tests::the_scenery_brush_places_its_sprite_and_the_eraser_takes_it_back` |
| The front door's named keys open the named activity | `app::splash::tests::the_named_keys_open_the_activity_they_name` |
| Any key/click still dismisses, never before `ARMED` | `app::splash::tests::a_key_or_a_click_opens_the_door_and_an_early_one_does_not` |
| A goal answers to both of its cells | `sim::level::editing_tests::a_goal_is_addressed_by_its_flag_as_well_as_its_pole` |
| A tree is erased by its canopy | `sim::level::editing_tests::a_tree_is_erased_by_its_canopy_as_well_as_its_trunk` |
| The eraser takes a tree by its leaves | `app::editor::tests::the_eraser_takes_a_tree_by_its_canopy` |
| The eraser takes the flag the user can see | `app::editor::tests::the_eraser_takes_a_goal_by_the_flag_the_user_can_see` |
| The ghost and the document agree the goal is already there | `app::editor::tests::the_ghost_and_the_document_agree_that_the_goal_is_already_here` |
| The flag is drawn in the cell the eraser addresses | `app::view::tests::the_flag_is_drawn_in_the_cell_the_eraser_addresses` |
| The flag can be erased, and a document without one says so | `app::editor::tests::the_goal_can_be_erased_away_and_the_editor_says_what_that_costs` |
| Placing a goal moves the one that is there | `sim::level::editing_tests::placing_a_goal_moves_the_one_that_is_already_there` |
| A level has one flag, in the document and in a file | `sim::level::editing_tests::the_goal_can_be_erased_away_and_there_is_never_more_than_one` |
| A file with two flags (or a `G` beside one) is refused | `sim::tests::levels::malformed_levels_are_refused` |
| Both directions offer the same gears (the sprint is not a rightward gear) | `sim::physics::tests::the_two_moving_directions_offer_the_same_gears` |
| The mirrored sprint is a second speed, not a second name | `sim::physics::tests::a_sprint_covers_more_ground_than_a_run_in_both_directions` |
| Shift + either arrow is that direction's sprint, in the shared keyboard mapping | `render::play::tests::held_intents_map_to_the_actions_in_priority_order` |
| The gap slots describe the ground beside each foot, not the way the body is going | `sim::tests::sensors::the_gap_slots_ignore_the_direction_the_body_is_moving` |
| The two gap slots are mirror images of each other | `sim::tests::sensors::the_gap_slots_fire_on_the_side_the_ground_runs_out` |
| The observation layout is what it says it is, at 95 inputs and version 4 | `sim::tests::sensors::the_vector_has_the_documented_named_layout` |
| The whole observation vector is pinned (re-pinned once, for v4) | `sim::tests::sensors::golden_observation_hash_at_a_fixed_state` |
| A mirror is the same level reversed (tiles, spawn, flag, scenery) | `sim::tests::levels::every_shipped_levels_mirror_is_reachable_in_principle` |
| Every shipped level's mirror is still beatable in principle | same test (the oracle on all nine) |
| A mirrored level is still *learned*, inside the level's own budget | `evo::tests::difficulty::a_mirrored_level_is_learned_within_the_same_budget_as_the_level_it_mirrors` |
| The network's shape follows the sim's tables, not a hand-written number | `evo::genome::tests::default_shape_matches_the_plan` |
| A fine child moves one to three genes of the best genome, not eighty-six | `evo::ga::tests::fine_children_are_local_steps_from_the_generations_best` |
| A run that never finds a new best keeps its step instead of freezing at the floor | `evo::ga::tests::a_stagnant_run_keeps_its_step_instead_of_shrinking_it_to_nothing` |
| The step rule grows, shrinks, ignores a thin sample, clamps, and holds on a plateau | `evo::ga::tests::the_one_fifth_rule_reads_the_success_rate_of_the_children` |
| Fresh genomes arrive on the stagnation threshold, and only then | `evo::ga::tests::immigrants_arrive_only_on_the_stagnation_threshold` |
| The search's own state is reproducible from the master seed | `evo::ga::tests::the_search_bookkeeping_is_as_reproducible_as_the_rest_of_the_run` |
| The panel's search line reports the step and only what it has earned | `app::panel::tests::the_search_line_says_the_step_and_nothing_it_has_not_earned` |
| A reheat moves every gene and keeps the genome's shape | `evo::genome::tests::a_reheat_moves_every_gene_and_keeps_the_genomes_shape` |
| The batch's temperature is the run's step times the factor, capped at the sigma ceiling | `evo::ga::tests::the_reheat_temperature_is_the_runs_own_step_scaled_and_capped` |
| A batch is really thrown at the temperature the config asks for | `evo::ga::tests::a_plateau_batch_is_thrown_at_the_temperature_the_config_asks_for` |
| Even the hottest batch is still a pull towards the champion | `evo::ga::tests::even_the_hottest_batch_is_still_a_pull_towards_the_champion` |
| A flag the user did not give leaves the trainer's default alone | `train_cli::commands::tests::the_command_lines_defaults_are_the_trainers_defaults` |
| A champion without the search knobs parses, and the reheat factor defaults to 1.0 | `evo::champion::tests::a_champion_file_without_the_search_knobs_still_parses` |
