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

### 2.4 The nearest-enemy sensor sees through the whole level — **fixed (v5, v6), and extended (v7)**

Finding 17, as it stood: `sensors::observe` gave slots 81–82 the nearest live walker's
`dx/dy` clamped at a view distance, so a walker 80 tiles away read exactly like one on top of
the body, and any level with a walker handed every agent a constant "enemy that way" input
from tick 0.

Fixed in two steps, both measured: **v5** bounded the vector at four tiles, so a distant walker
reads `(0.0, 0.0)` and the slots mean "no enemy in sight"; **v6** widened the bound to eight
after four was measured to be *worse to learn from* than the unbounded lie (twelve seeds on
`l3-gauntlet`: mean first win 41 generations at four tiles, 25 unbounded, 18 at eight) — a
threat that is only legible once it is inside the patch is a threat met at contact range. The
pass after that appended what the vector was still missing, and the thing the level format
cannot supply: **v7's `enemy_dir`**, the nearest sighted walker's own direction of travel. That
last slot is what makes a patrol *timeable* rather than merely locatable, and §4 measures it on
the level built to need it.

### 2.5 `--trials` is off by default, so walker levels are noisy — **fixed this pass**

Finding 14 established that `episode_seed` mixes in the generation, so on the one
stochastic level an elite re-rolls the walker's patrol phase every generation and can
lose the run that won. `--trials` averages that noise but defaults to 1.

For two passes the fix sat in the tree as advice nothing called: `GaConfig::recommended_trials_for`
existed, was documented, and **had no caller**, while `train_config` hard-coded its own `3` beside
the flag name — the same defect this file records for `--reheat-factor` in §4, and the same
consequence: the window, a resumed run and the API judged a walker level on a **single** patrol
phase while the command line judged it on three. The rule now lives in the trainer
([`GaConfig::effective_trials`]), every front-end asks it, and it is **three** phases on a level
with walkers and one on a level without — three rather than two because two is the thinnest
sample that can show a genome is unreliable, and because three is what the command line was
already doing, which makes the tables taken through the CLI comparable with the ones taken through
the window. `--no-auto-trials` releases the rule and pins the literal count (reproducing the
one-phase protocol every table in this document written before this pass was taken with). See §4
for the measurement.

### 2.6 No automated check runs the determinism suite — **fixed this pass**

The determinism contract is the spine of the project and there was no `.github/`
workflow (or equivalent) to run `cargo test --workspace` on a change. The tests were
excellent and nothing enforced them.

There is now a workflow ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) on every push
and pull request: `cargo clippy --workspace --all-targets -- -D warnings` and then
`cargo test --workspace`, both unfiltered and neither with an escape hatch. The clippy step
carries `-D warnings` because the tree is clean at that bar today and a lint bar that is not
enforced is a lint bar that drifts; the test step carries no `--skip`, because a filtered run is
not the suite. The header comment names what is being protected — the sim's `std`/thread/wall-clock
rules, the thread-count-independent RNG stream, and the version trio that refuses a stale
champion — which is the part a reader a year from now will wonder about.

What it does not do, and should not be read as doing: it cannot check that the *numbers* in this
document are still true (those are measurements, and a machine that re-took all of them on every
push would be measuring its own runner), and it is light enough that a full cold run fits in the
job's `timeout-minutes: 30` with the toolchain cache.

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

### 2.9 A walker has a position but no heading — **half wrong, and re-measured this pass**

This was written as: `Walker::new` builds every walker facing right, the level file records only
where a walker stands, so "a patrol's first direction is a constant", and on a mirrored level that
constant is an ambush — measured as first-win generations 4 / 3 / 7 / 11 against 18 / 18 / 17 / 16
on `l8-long-jump` and its mirror, with the walker deleted the pair going 1 / 1 / 2 / 1 against
5 / 0 / 5 / 3.

**The premise is wrong about the sim, and the measurement does not reproduce.** A walker's initial
direction is drawn from the episode RNG in
[`Walker::spawn_spread`](crates/sim/src/physics.rs) — that draw is the sim's only gameplay
randomness and the reason a walker level is stochastic at all — and it has been there since the
repository's first commit. `Walker::new` does build a walker facing right, but that constructor is
the deterministic pose tooling asks for; nothing in an episode goes through it. Re-run this pass,
pop 200, seeds 1-4, `--stop-on-win`: **4 / 4 solved at 7, 3, 3, 4 against 3, 0, 5, 6** (means 4.2
and 3.5), and with the walker deleted 2, 2, 3, 6 against 3, 0, 3, 5 (means 3.2 and 2.8). The
mirror is the *faster* arm in both pairings, at both ends of the walker ablation, and the
six-of-six gap is gone. The level format's silence about direction is therefore not a defect to
be repaired by a `heading` field: an authored heading would make every trial of a walker level the
byte-identical episode and quietly delete what `--trials` exists to average.

**What was genuinely missing is the other side of the same fact: the body could not see which way
a walker was walking.** That went in as sensors **v7** (`enemy_dir`, index 95 — see §4 and the
module docs in `crates/sim/src/sensors.rs`), which is an observation change rather than a level
format one, and it is the version that makes this item survivable: the patrol phase stays
deliberately unpredictable and timeable rather than authored.

### 2.10 A checkpoint was only credited for a crossing on the level's *left* end — **fixed (fitness v6)**

`Level::landing_zone` + `Level::zone_is_checkpoint` decided whether arriving on a stretch of
ground meant crossing something by looking at the tile to the left of the zone's start. On
a rightward level that is exactly the right question. On a leftward one the crossing is at
the zone's *right* end, so a landing that should pay could pay nothing — the last rightward
assumption in the *reward* (the observation's went in 1.9, and the frame cap in 1.8).

Fixed as the fix's own note predicted it would have to be: the sim now hands the zone the
**arrival side**. `Level::checkpoint_zone_from(tx, surface_row, from_col)` credits a crossing
at whichever end the body came from (`from_col < start` looks left of the start, `from_col > end`
looks right of the end, and a body arriving from *inside* the zone crossed nothing), and the term
is symmetric by construction rather than by a second rule.

Two things are worth keeping from the unfixed version of this entry, because the cheaper answer is
still the tempting one. The obvious symmetric-looking rule — "credit a zone if *either* end is a
crossing" — is **wrong**: it would pay a body for standing on the near side of a pit it has not
crossed yet, which is the one thing this term exists not to do. And the size is why this was low
priority and stays low: [`CHECKPOINT_BONUS`] is one tile (`16`) per zone against a goal worth
`10 000`, so the defect was worth at most a few dozen points on the levels that have that many
zones. The version bump is what it cost — the term *did* pay on the shipped mirrors, so every score
in the pack with a hazard on the far side of a landing zone is a different number now, which is
exactly why [`FITNESS_VERSION`] moved to 6 rather than the change going in quietly. Both halves are
pinned in
`sim::tests::physics::a_landing_zone_is_a_checkpoint_only_when_something_had_to_be_crossed`: the
rightward clauses it always had, and a mirror clause asserting the far side of a *mirrored* spike is
credited when the body arrives from the right — the case that used to pay nothing.

### 2.11 The Open menu lists raw filenames and there is no starter pack

The README already says M4 is missing "the curated starter pack and a browsable level
shelf". A new player sees `l1-flat-run.json … l7-wall-school.json` and a
`scratch-l1-walkers.json` with no "start here".

### 2.12 A spike killed 0.1875 tiles above its topmost drawn pixel — **fixed (0.10.0)**

The spike sprite leaves the top three of its sixteen pixel rows transparent, deliberately: a
spike tile drawn to its ceiling reads as part of the wall above it rather than as a hazard
(`tools/art/terrain.py::spikes`, `apex = 3`). The hazard check tested the **whole tile**
anyway, so every spike carried an invisible kill band 0.1875 tiles tall above the topmost
pixel of it — and a body can clear every pixel of a spike and still die. Reported from the
window as "the hitbox seems to over-extend when the agent is standing beside the spike"; the
stand-beside case is fine (a body whose box ends at the cell's edge is in the next tile
over and is not tested), but the band above the tips is real and it was costing levels.

**Measured** on the level that produced the report — a hand-drawn 60×15 whose only route
onto a four-wide platform crossed a spike crowning that platform's *first* column, with a
one-tile stepping stone as the take-off: sweeping 81 sub-tile take-off positions across the
stone against 80 lengths of jump hold (6 480 held-jump-then-run pairs), take-offs that reach
the flag alive were

* **x = 39.000 only** under the tile rule — one position out of 81, a window 0.005 tiles wide; and
* **x = 39.000 .. 39.130** under the drawn box — 1 846 live pairs against 67, twenty-seven
times the positions.

Before the change, 4 563 of those 6 480 pairs died while the body's lowest point inside the
spike's column was still above the topmost drawn pixel: they never touched a pixel of the
spike. The same level's champion — the one the report pointed at — stalls at the platform's
face instead, which is a level-design problem rather than this one and is recorded here so
the two are not confused: from a take-off flush against that face the body's lowest point
inside the spike's column is 12.762 while the hazard demands 12.000, and no jump from that
spot can ever clear it. The fix widens the band you can work in; it does not make a spike in
the only entry to a platform reachable.

The hazard is now box-versus-box against [`physics::spike_lethal_box`](crates/sim/src/physics.rs)
— the lower 0.8125 tiles of the cell, the sprite's own extent — and the oracle's
"a move cannot begin inside a hazard" guard asks the same, weaker question the game asks
(`reach::hazard_start`), so the sweep cannot invent a false negative out of the band the
sim just freed. The drawn triangles taper to points, so a body clipping the blank air
*between* two tips in that top band is still killed: box versus box is the honest
approximation for one tile type, and it is the one place the rule is stricter than the
drawing.

Physical, so the trio moves: the workspace is **0.10.0** and every champion trained under
0.9.0 is refused at load rather than replayed against physics it was not trained on. The
long-runway golden hash is unchanged (there is no hazard on that runway) even though the
version pin moved, which is the coverage gap `tests/episodes.rs` already documents.
Pinned by `sim::tests::physics::a_spike_kills_where_it_is_drawn_and_not_in_the_air_above_its_tips`
(verified to fail under the old tile rule), which holds both halves: the band above the tips
is air, and a body standing in the spike's own cell is still killed — the assumption
`Level::is_standable`, the checkpoint zones and the editor's placement rules are built on.

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

### The window's own furniture: a dock, and one vocabulary for the things you press

The other half of the request was the *look*, and the diagnosis was specific: the window was
opening on **egui's stock buttons lying flat on stock panels**, which is what a tool looks like
rather than what a game looks like. Nothing about a row of grey rounded rectangles says *this is a
level editor*, and the level behind them was contradicting them.

What landed is one vocabulary, in one place, used by everything the pointer can press.

* **`theme` grew the depth it was missing**: `DOCK` (the bar itself — a step up from a panel and
  warmer than it), `BEVEL_LIGHT`/`BEVEL_DARK` (the lit and shaded edges of a raised face),
  `PLATE` (a sign: the level's name, a banner), `SHADOW`, a second corner radius (`RADIUS_LG` = 8
  for things you *press*, against the 3 that keeps a chart's frame honest) and `PRESS_SINK` = 2,
  which is the whole of what "tactile" means at these sizes.
* **`crates/app/src/ui.rs` is the vocabulary itself** (new module). `bevel(PlateState, Rect) ->
  Bevel` is a *pure function* — no `Ui`, no painter, no input — resolving one control's state and
  rectangle into every colour and every sub-rectangle it needs, which is what makes the look
  *assertable* rather than merely screenshot-able: a pressed plate's content sits exactly
  `PRESS_SINK` below a resting one's and the **face** does not move (that is what "sinks into its
  socket" means); a pressed plate **inverts** its edges, so the shadow moves above the light; the
  five states — disabled, resting, hovered, pressed, selected — have five different faces, and the
  selected one wears `ACCENT` with `SHELL` type on it; and a disabled plate refuses the pointer
  rather than drawing feedback for a press it will never answer.
* **`chunky_button`, `segmented`, `plaque`, `banner`.** A tool is an icon above a name on a plate.
  The mode switch is *one object*: a socket holding four exclusive cells with the chosen one
  standing out of the group, and a cell whose word will not fit beside its icon keeps the word and
  drops the icon (`segment_label_room`, with a test that measures the longest word — *Evolve* — in
  the font it is drawn in). The plaque is a two-line sign; the banner is a plate, a lamp and one
  monospaced line, and it *measures itself against the room it is given and declines to draw at
  all* when that room is too small, which is what keeps a reading from being cut in half by a clip
  rectangle.
* **The dock is built out of them.** The title bar is now the level's name chalked on a sign (with
  the file and its unsaved dot under it), the five document tools — Levels, New, Save (gold while
  the canvas is ahead of the disk), Undo, Redo — the mode switch carrying the game's own art (a
  ground tile for Edit, the runner for Play, a walker for Evolve, the flag for Replay), *Restart*
  beside it only while a person is playing, *Fit* to frame the level, and at the right end the run's
  own reading (`gen 43/300  best 218.2  wins 2/200`) with a lamp that is green on a win, amber on a
  stall and gold while it is still improving. The icons are **atlas sprites**, so the tool bar is
  made of the same sixteen-pixel pictures as the level behind it — the one thing a text label
  cannot do.
* **Two rules the dock keeps that a row of buttons does not.** Its right-hand end is *dropped, not
  clipped*, when the window is narrow, and the file path that used to sit there is gone entirely:
  the plaque already names the file, and a bar that says the same thing at both ends is spending the
  middle of itself on a caption. And all five tool responses are collected *before* any of them is
  acted on, so a save cannot move the levels directory out from under the *Levels…* button
  mid-row — the kind of flicker that reads as a prototype.
* **The palette wears the same plates.** Brush cells and the scenery picker used to draw their own
  rectangles — `CARD` with a hand-rolled ring, four colour literals in the picker including
  `250, 214, 102` written out by hand — so the thing you paint with and the thing you open a level
  with answered the pointer differently. They go through `bevel` now: hover lifts, press sinks, the
  selected brush is the accent with its picture still legible on it, and the picker's literals are
  theme constants.

**What this does not claim: animation.** There is none, on purpose. A plate's state is a fact about
where the pointer is, and a spring-damped bounce at forty points reads as a web page rather than as a
game.

*(Two sentences above are superseded by the pass that follows: the switch no longer carries the word
of every cell — only the chosen one's — and so the cell-width clearance it was measured against,
`segment_label_room`, no longer exists. The five tools are six, and none of them carries a caption.)*

### The look, second pass: fewer words, one window, and a theme that was never installed

The report that opened this pass was "still cluttered, still carrying a lot of information, not the
graphical feel of something like Super Mario Maker 2", and the honest reading of the previous pass is
that it had dressed the furniture without removing any of it: the *words* were all still there. Two
researches were run before touching the code — SMM2's Course Maker specifically, and game-HUD design
guidance generally — and what they agree on is worth stating because it shaped everything below:
icon-first tools with the meaning on hover, a very small number of words on the editing screen, one
parts strip rather than a properties pane, and controls that read as physical objects in a chunky,
rounded, *outlined* style ([Play Nintendo on the parts strip](https://play.nintendo.com/news-tips/tips-tricks/super-mario-maker-2-tips-tricks/),
[Shacknews on the docked controls](https://www.shacknews.com/article/112648/docked-controls-in-super-mario-maker-2-course-maker),
[Nintendo Life on the parts menu](https://www.nintendolife.com/guides/whats-new-in-super-mario-maker-2-for-nintendo-switch)).
The research is thin on *measured* values — no accessible source gives SMM2's panel colours or cell
sizes, and the report says so plainly rather than inventing them — so the palette stayed the game's own
and only the *structure* was taken.

**One plate size, and no captions.** Everything the pointer can press in the dock is one 52-point
square (`ui::TOOL_SIZE`); the second size for label-only controls is gone, because there are no
label-only controls left. `chunky_button` became `icon_button`: a glyph, and a name that is drawn
only when the atlas cannot supply the glyph (a tileset from before the glyphs still runs, with words
on its plates). What a control is *called* is in its tooltip, which is where a game's own bar puts it.

> **Reversed in the third pass.** The plates carry their captions again and so does every cell of the
> switch — see *a name on every control* below. The reading of the research was right about the
> *number* of words on the screen and wrong about zero; the state a person most needs to read the
> window in is the one where they have pressed nothing yet.
The dock's tool row is therefore six pictures: the signpost for the pack, the spawn pad for a new
level, the crate a save writes, a turn arrow each way for the stroke history, and a magnifier for the
level's own numbers.

**Five glyphs were drawn for the four controls the world has no picture of** (four more followed in
the third pass, for the run's own controls: `ui_pause`, `ui_play`, `ui_step`, `ui_medal`).
`tools/art/ui.py` is a new art family — `ui_undo`, `ui_redo`, `ui_fit`, `ui_info`, `ui_restart` — drawn geometrically rather
than as 16-row string art, because an arrow is a *relationship* (the two turn arrows are reflections
of one arc and one head, so they cannot drift apart by a pixel) and because a glyph is one flat ink in
this palette, with the plate carrying every bit of state. They are the first sprites in the atlas the
game itself never draws, they sit before the seams (whose row must stay last, for the reason that
family's own comment gives), and they took the grid from 16x14 to 16x15 — 223 of 224 cells was a full
atlas, not a growable one. They are *optional* art: `ui::icon_button` falls back to the control's name,
which is exactly why `app::atlas::tests::every_glyph_the_window_draws_is_in_the_atlas` exists — a
renamed glyph is a bar of small words, not an error.

**The switch names only the cell that is on.** `ui::segmented` now allocates the chosen cell from its
own label, measured in the font it will be drawn in, and draws the other three as bare pictures: four
independent words became one. That inverts the old failure mode instead of guarding against it — a cell
sized *by* its word cannot clip it — so the test that used to measure *Evolve* against a clearance
became a test that the rule gives every one of the four words its own width, and that the widths
differ (a rule returning one number would have passed the old assertions). The third pass labels all
four cells and sizes them from the *widest* name in the set, so the group no longer changes shape with
the choice.

**The editor's properties column is gone.** This is the largest simplification in the pass, and it is
the one the SMM2 research asks for directly: an editor screen has no side panel of readings. The
verdict — *can the spawn reach the flag* — moved into the strip along the bottom as a lamp and a word
(shared by both places that show it, so the two cannot disagree), what was behind it became a floating
**Details** window that the dock's magnifier toggles, and the canvas took the whole width. The zoom
reading moved in there too; the status strip's right-hand end keeps only what a person asks *while
drawing* (the tile under the pointer, anything wrong with the atlas). The run views keep their panel,
and for a stated reason rather than by accident: that panel holds the controls a run is driven with, so
it is busy by nature rather than empty by default.

**The palette says one thing.** It used to say three: a bare `Parts` heading, a line of instruction
about dragging, and the selected brush named under the grid. It is now one line that answers the one
question a person has of a palette — `PARTS · GROUND` — and the instruction lives on each cell's hover,
where somebody asking "what is this cell" is already looking.

**The editor's ground stopped being a transparency checkerboard.** The canvas drew alternating pale-blue
cells over the sky gradient, which is the picture an image tool paints where there is *nothing*; every
cell of it was saying that about the one surface in the window that is full of a person's work. It is
a flat work-surface blue now, with the tile grid drawn on it — the grid lines were being drawn all
along, and they are what an eye actually reads tile boundaries from. The level's own art gets the whole
of the contrast.

**And the defect the renders found, which was not a look at all.** The first frame with the details
window open came out **dark** — egui's stock window frame and type inside a cream window. The cause is
worth writing down because it invalidated the previous pass's verification as much as this one: egui
keeps *two* styles, one per theme, and chooses between them from the system's preference, while
`Context::set_style` writes only into whichever is selected at the time. The theme was installed into
the **light** slot and this machine reports a **dark** desktop, so the installed theme was never used by
anything egui styles *itself* — a floating window, a button, a slider, a separator — while every surface
that looked right (the panels, the dock, the canvas) was right only because it paints its own fill.
`theme::apply` now pins the preference and writes **both** slots, and the new test asserts the slot that
nothing was using.A pass that had "verified" this half by looking at panels had been looking at
the exception rather than the rule.

### The look, third pass: a name on every control, and a run's panel as a menu

The report that opened this pass was two lines: the Edit, Play and Evolve buttons "do not have text
labels until they are clicked", and on the Evolve screen "there is still a LOT of information being
thrown at the user". They are one mistake made twice — the second pass took "few words" and read it as
"no words", and left a run's panel saying everything it knew at once — and they have one fix: decide
what the window *has to* say, and put the rest one press away.

**Every control says its own name.** All four switch cells are labelled, not just the chosen one, and
every dock plate carries its caption under its picture. Three bare pictures in a row are a puzzle: the
state a person most needs to read the window in is the one where they have pressed nothing yet, and a
picture of a walker is not a guess at *Evolve*. The switch's four cells are one width, taken from the
widest name in the set and measured in the font it is drawn in, so the bar does not twitch sideways
when the mode changes; the tooltips keep the sentences a word cannot hold. Four new glyphs
(`ui_pause`, `ui_play`, `ui_step`, `ui_medal`) give the run's own rows pictures to put beside their
words, and `ui::menu_row` is the second half of the vocabulary — a wide short plate with the glyph at
its left, which is what a column of controls wants, against the dock's glyph-over-word.

**The Evolve screen is a menu, and the instrument board moved behind the magnifier.** A run with any
age on it put thirty-odd lines beside the level: the search's sigma and its stall counter, the
diversity of the archive, the x the population kept dying at, a genius note, a curriculum note, the
death cluster, a best-agent block, a row for every agent in the generation, the recorded champions and
their scrubber, two sliders and three checkboxes. All real, and read at a glance the panel said
nothing — the question a viewer watching a population learn to jump asks is *is it getting anywhere*,
and the answer was buried under eleven other readings. It is now **three readings and one word**:
which generation of how many, how many of the population reached the flag, the best fitness, and the
trend — plus five rows (pause/resume, one generation, save champion, restart, details) and two
conditions that have to be *told* rather than shown (the level has been edited since the run started;
the run is over). The replay's panel is the same shape, with its toggles as plates that go gold.

**Nothing was deleted.** The **Details window** is mode-aware: the level's numbers and verdict in the
editor, and a run's board in Evolve and Replay — the step and the stall, the diversity, the
generation's max/avg/min, the death cluster, the champion's own record, a row per agent in the field,
the recorded champions with their scrubber, the two ladders and the key list. A viewer learns one
control and gets whichever answer belongs to the mode they are in. The window scrolls, and the field
list is capped and scrolls inside it, because a population of two hundred ghosts is taller than a
display. Both panels and the window report the same `panel::Actions`, so a pause is a pause wherever
it was pressed.

**And the duplicated readings went with it.** The dock's banner carries no run's numbers at all now —
it was reading out the same generation, flag count and best fitness the panel shows two hundred points
away — and the evolve status line is "gen 40 · 8 of 8 running · 8x · seed 7" instead of a sentence that
spelled out the field and timed the animation to a tenth of a second. The replay's line lost the ghost
roster for the same reason and kept the half only it can state (a hidden field says it is hidden).
Those two lines are the longest in the window, and they are now **truncated** rather than allowed to
push everything after them — the tile under the pointer, the warning — off the edge.

**Verified by rendering and looking**, which is how every defect in this pass was found rather than a
check: `target/shots/v5-evolve.png` (a finished run), `v5-replay.png`, `v5-evolve-details.png`,
`v5-replay-details.png`, and the 900-point minimum in `v5-narrow-edit.png` / `v5-narrow-evolve.png` /
`v5-narrow-replay.png`, where a labelled switch, a wider dock and a 300-point panel still fit. What is
*not* verified is any of this with a pointer in hand: there is no display here, so the feel of pressing
a menu row, and Play mode's own dock (which has no scripting flag and so was never rendered), rest on
the headless widget tests and on the render.

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

### Where the loop is capped now, ranked (this pass's review)

Three passes have fixed the *fitness landscape* and the *observation*, and both are in good
shape: checkpoints, a priced jump, a distance-to-goal score, an enemy vector that does not lie,
no phantom inputs, and a decision cadence that was measured rather than guessed. What is left is
not a bug list. It is the list of things that still stand between this population and behaviour a
person would call *smart*, ordered by how much each one actually limits that — with the cost and
the evidence for each, because the next pass should not have to re-derive this.

1. **The tasks are the ceiling, not the search.** The shipped pack is solved almost
   immediately: `l1-flat-run` and `l2-first-gap` at **generation 1-2** at pop 200, and
   `l6-marathon` — the longest level in the pack — is won at generation **6 of 300**, by a
   population that has not learned anything yet. That is a lottery, not a policy: nothing about
   the level forces a chain of decisions, so nothing about the level forces the *population* to
   become sophisticated, and a run that solves its problem in six generations is being asked a
   very easy question. Emergent behaviour comes from long horizons and interacting mechanics,
   and this pack has one hazard type (spikes), one enemy type (walkers that always set off
   rightward, finding 2.9), and static terrain. **The cheapest large gain available is authoring
   harder content**: a level that needs a wait, a climb and a hazard crossing in sequence, with
   the flag far enough away that no reflex reaches it. Everything below makes such a level
   *learnable*; none of it substitutes for it.
   **Done this pass.** `levels/l9-wait-and-climb.json` is that level — a chain of nine moves, with
   the spawn at column 2 and the flag at column 56 — and it moves the pack's hardest first-win generation
   from **20** (L3, the old slowest median) to a median of **160**. See "Content" below.
2. **The policy had no memory.** Fixed in this pass, and measured below — the one change here
   that adds a *capability* rather than a gradient. Without state a decision is a pure function
   of the frame, so "hold this jump for three blocks", "wait for the walker to turn" and "this
   is the second attempt at this ledge" are not merely unlearned, they are unexpressible.
3. **Selection reads one episode.** `GaConfig::recommended_trials_for(level)` — and the argument
   in finding 2.5 for using it — has been in the tree since the trials work and **nothing calls
   it**: on a level with walkers every genome is judged on a single patrol phase, so an elite
   can be re-rolled out of the run by a phase it never saw (finding 14), and the pack's longest
   level is "solved" by one lucky agent in six generations. Two or three trials on the levels
   that are stochastic is the cheapest honest fix in this list, costs no version bump, and makes
   every other number in this document more trustworthy — including the guesses in it.
   **Done this pass, and the measurement is a caution rather than a confirmation.** The rule is
   wired everywhere and the number is three; what it buys on the two levels the pack actually
   ships is *nothing measurable in solves* at 2.4x the wall clock, because both levels are won by
   the population in ten generations whether a genome is judged on one phase or three. It does
   not even remove L6's lottery, because a lottery on a level that easy is the level's fault
   rather than selection's. See "The trial rule" below for the numbers.
4. **Diversity is measured but never selected for.** The MAP-Elites archive fills every
   generation and is reported, and breeding from it was measured as a loss on the plateau, so it
   is off. Nothing anywhere *rewards* behaving differently: an elitist tournament with crossover
   walks the whole population into one lineage early, which is why every hard level in this pack
   is a lottery — twelve seeds of one level produce twelve different answers. The untried version
   is the reward-level one (a novelty or diversity term, or a diversity-*triggered* breeding rule
   hung on the stall detector that already exists), which is a different mechanism from
   parenting out of the archive and is not refuted by the archive's measurement.
   **The gated version is built and measured this pass, and it is the first archive reading that
   is not a loss on a plateau**: 9 of 12 seeds at a mean first-win generation of 76, against 10 of
   12 at 94 with the archive off and 7 of 12 at 121 with the ungated trickle. It costs the hazard
   floor the one reading the trickle earned (3 of 12 down to 1), so it stays a flag rather than a
   setting. See "The archive gate" below.
   **The reward-level half is now built and measured as well** (`--novelty`, a rarity bonus added
   to the score the population is ranked and bred by and to nothing that is recorded), and the
   answer is that a small bonus is a real mechanism with a narrow window: it is worth a few seeds
   on the plateau, worth nothing on the chain level, and worth nothing measurable at any weight
   tried on the hazard floor. See "The rarity bonus" below for the table and for why `0.0` is
   still the default.
5. **One brain, one level.** Every claim in this document is about a specialist: `Trainer::step`
   takes one level, and a champion is played only where it was trained (the splash documents
   this as a rule). Nothing generalizes, and nothing is *asked* to — but `PopulationFile` already
   moves a whole population between levels for the curriculum, so a suite trainer (a generation
   scored on two or three levels) is a small step from machinery that exists and is the standard
   route to a policy that looks clever rather than memorised. **Built and measured this pass**, as
   `train --suite a.json,b.json,...` plus `score` (which asks a saved champion the same question),
   with one substantive deviation from this list's sketch: the aggregate is the genome's **worst**
   level rather than a sum, because a flag is worth `10 000` and a whole level of progress about
   `1 000`, so a sum lets the easiest level own the score and teaches exactly the specialisation a
   suite exists to remove. The measurement is narrower than the item hoped and the honest headline
   is the *shape* of the result rather than its size: selection on a suite does not make each level
   fall faster, it makes one policy hold several at once, which is the thing no arm in this document
   could do before. See "The suite: one policy, three levels" below.
6. **The observation has two documented blind spots.** A walker has no heading, so its patrol is
   unpredictable in the one way that matters (2.9, and the level format has no field for it), and
   a walker beyond eight tiles is invisible (deliberate, v5/v6). The 9×9 patch is the documented
   upgrade path, gated on "evolution visibly plateauing on levels that should be solvable" —
   which, given 1-2-generation solves, it is not yet.
   **Half done this pass, and the other half is now a decision rather than a gap.** The *body*
   can see a walker's direction as of sensors **v7** (`enemy_dir`, index 95); the *level file*
   still cannot say it, and 2.9 is where the reason is written down — an authored heading would
   make every trial of a walker level the same episode, which is the mechanism this pass spent
   part of its budget measuring. The patch stays the gated upgrade path, and l9 is the first level
   written to need the horizon rather than the patch: nine moves in sequence.
7. **There is no independent baseline.** The GA is compared only against its own earlier
   versions. An ES or CMA-ES arm on the same episode budget and the same levels would say
   whether the search or the problem is the binding constraint, and it is the one measurement in
   this list that can invalidate the others.
   **Built and measured this pass: `train-cli es`.** On the same episodes it solves 0 of 12 seeds
   of `scratch-tower` and 2 of 12 of `l3-gauntlet` where the GA solves 10 and 12 — so the search is
   *not* where the remaining work is either: the GA's breeding is earning its place, and the
   constraint on this pack is the content, which is item 1. See "The independent baseline"
   below.

**Order to do them in:** 3 is nearly free and improves the honesty of every later measurement,
so it comes first (with 6's walker heading, which is one field and one sensor slot); then 1, the
content, because a harder level is what makes 2, 4 and 5 measurable at all; then 5 and 4 as the
mechanisms that a harder level will need. 7 is worth doing before any large investment in the
search, because if a plain ES matches the GA at equal episode budget, the search is not where
the remaining work is.

**Where that order stands now.** 3 is wired (and its measurement says it was worth less than
this list assumed), 6's observation half is in as v7, 7 is built and answered, 4's gated half is
measured, 1 landed as a level that is nine moves long, and 2's memory question is answered below
(yes in generations, no in solve count). **That leaves 5 and 4's reward half, and this pass did
both**: a suite trainer with a `score` command that checks the claim on a saved artifact, and a
rarity bonus in the selection score, off by default and measured on the two levels that would
show it. Every item on this list now has a mechanism, a measurement and a paragraph saying what
the measurement does *not* establish — which is the point at which the next pass should be
re-ranking the list rather than working down it, and the two sections below are written so that
re-ranking can start from the numbers rather than from the prose.

### Memory: the policy can now have a state of its own

`NetShape` gained `feedback`: that many values of the network's own first hidden layer are
concatenated onto the next decision's observation, and a fresh scratch means a fresh, empty
state, so a recurrent policy is exactly as reproducible as a memoryless one (the episode is still
a function of genome, level and seed). `--feedback <N>` exposes it; 0 is the default and is
bit-for-bit the net every earlier pass measured. Eight values of sixteen cost 128 genes (1 723 →
1 851, +7 %) and, on a level where the population is running long episodes, about a third of the
wall clock.

The measurement is the interesting part, because memory does *not* pay uniformly — it pays exactly
where a decision has to survive past the frame that prompted it. `--feedback 0` against
`--feedback 8`, pop 200, 300 generations, `--stop-on-win`, first-win generation:

| level | seeds | memoryless | with memory (8) |
|---|---|---|---|
| `l2-first-gap` | 12 | 12/12, mean 2 | 12/12, mean 1 |
| `l4-rhythm` | 12 | 12/12, mean 11 | 12/12, mean **7** |
| `l5-tightrope` | 12 | 12/12, mean 16 | 12/12, mean 20 |
| `l3-gauntlet` (walkers) | 12 | 12/12, mean 18 | 12/12, mean **14** |
| **`scratch-spike-tight`** (the hazard floor) | **24** | **2/24**, mean 145, solved at 117 and 173 | **11/24**, mean 134, solved at 28, 68, 70, 99, 99, 115, 160, 178, 180, 227, 251 |
| `scratch-tower` (the plateau) | 24 | 14/24, mean 94 | 13/24, mean 137 |

**Every number in this table is a v6 reading** — 95 inputs, 1 723 genes — because it was taken
before the sensors' v7 slot existed, and it is recorded rather than re-run: the same protocol on
the pack's first chain level, at v7, is in "Content" below, and the frozen protocol is what
makes the two comparable at all.

Read as two facts rather than one. **On the hazard floor it is a different search**: the level the
owner's report is built on — one-tile spikes with one safe tile between them, the shape that
produced the park trap — goes from 2 of 24 seeds to 11 of 24, and the solves it finds are often
four times earlier (28 and 68 against 117 and 173). A policy that can hold a commitment across
decisions is exactly what that level asks for, and it is the level class this document just said
the pack does not have enough of. **On the plateau it is a wash that costs time**: 13 against 14 of
24 is noise, but the solves arrive later (mean 137 against 94) and the same runs spend a third more
wall clock, because a recurrent episode is longer, the genome is 7 % bigger, and the zig-zag is
decidable from the frame — every one of its moves is a reflex, so state is genes the search has to
carry and can do nothing with. The default therefore stays 0, exactly like the archive trickle and
for the same reason: a mechanism that wins on the levels the pack lacks and loses time on the level
the pack is measured against is a knob, not a setting. `--feedback 8` is the first thing to reach
for on a hazard floor, a level with a wait in it, or anything with a jump that must be held.

### The independent baseline: an ES on the same episode budget

Item 7 above was the one item on the list that could invalidate the others, so it was built first of
this pass's experiments: `train-cli es`, a `(µ, λ)` evolution strategy over the same genome, on the
same level, with the *same* episode budget. Everything that decides what an episode is — the level,
the episode seed, the cadence, the memory width and the trial count — is set identically in both
arms; every search knob is not, because elites, tournaments and crossover describe a population the
ES arm does not have. `--pop` is λ on that arm and means the same compute: 200 episodes per
generation against the GA's 200 genomes per generation. Twelve seeds, pop 200, 300 generations,
`--stop-on-win`:

| level | GA | ES |
|---|---|---|
| `scratch-tower` (the plateau) | **10 / 12**, mean 94 | **0 / 12** |
| `l3-gauntlet` (walkers, 3 trials both arms) | **12 / 12**, mean 17.9 | **2 / 12**, mean 137 |

Paired on the seeds it is as lopsided as it looks: the ES gains nothing the GA misses and loses ten
seeds on either level (the two it wins on the gauntlet come at generations 32 and 242 against the
GA's 9-28). **So the search is not the binding constraint — the GA's breeding is doing real work,
and neither is it the thing standing between this population and intelligence.** Read with item 1
above, the two baselines agree: an ordinary ES cannot solve the pack at this budget, and the pack's
levels are solved by the GA in ten to twenty generations, so what is being asked is too easy rather
than what is answering it too weak.

The honest caveats, because a baseline is worth exactly its tuning. The ES arm runs at **its own
defaults** (σ = 0.05, lr = 0.01) and was not swept: a larger λ or a learning-rate schedule could do
better, and the literature's ES results are on problems where λ > n. At λ = 200 in 1 739 dimensions
the estimate is thin (`es.rs` says so where the arithmetic tests are justified), which is a statement
about the *batch size*, not about ES. What the measurement establishes is the question it was built
to answer — with this compute, on these levels, the GA is not replaceable by a gradient-free search
this simple — and no more than that.

Reproduce with `./target/release/train-cli es --level levels/scratch-tower.json --generations 300
--pop 200 --seed 1 --stop-on-win --quiet` and the matching `train` line, `--trials 3` on the gauntlet.

### The archive gate: the first archive reading that is not a loss on a plateau

Item 4 above named the untried mechanism: a diversity-*triggered* breeding rule hung on the stall
detector, as distinct from the every-generation trickle that was measured as a loss. It is built
(`--archive-on-diversity`, gating archive children on *no new best for `--stagnation` generations*
**and** no new behavioural niche over the same window) and this is its measurement, twelve seeds per
arm at pop 200, 300 generations, `--stop-on-win`, first-win generation:

| level | archive off | 2 children, every generation | 2 children, gated |
|---|---|---|---|
| `scratch-tower` (plateau) | **10 / 12**, mean 94 | 7 / 12, mean 121 | 9 / 12, mean **76** |
| `scratch-spike-tight` (hazard floor) | 1 / 12, mean 117 | **3 / 12**, mean 105 | 1 / 12, mean 192 |

Paired on the seeds: against the ungated trickle the gate gains seed 3 and loses 4, 6, 8, 9 and 12;
against the archive off it gains 3 and loses only 6 and 9. So on the plateau the gate **recovers
almost all of the trickle's loss and keeps its speed** — nine of twelve against ten, with the solves
arriving seventeen generations earlier on average — which is the first reading in this document where
the archive is not simply costing the hardest level in the pack.

**And on the hazard floor it is the wrong tool, which is why it is a flag.** The ungated trickle owns
the one level where it earns its keep (3 of 12 against 1, gaining seeds 7 and 11, losing none), and
the gate suppresses exactly that, because a run grinding across a hazard floor is a run that keeps
finding new bests and new niches — the condition it waits for never arrives. The rule of thumb the
two rows give is mechanical rather than mystical: **a plateau wants the gate, a grind wants the
trickle, and the default stays off for both.**

### Content: the pack's first chain level

Item 1 said the cheapest large gain available was content, and that a harder level is what makes the
memory and diversity mechanisms measurable at all. `levels/l9-wait-and-climb.json` is that level:
60×15, one walker, one flag, and **nine moves between the spawn and the flag** — spike, pit,
nineteen-tile patrol run, two-tile hazard patch, two-tile step, three-tile wall climb, a gap in the
upper ledge, a spike, and a two-tile climb, with the spawn at column 2 and the flag at column 56.
Every move in it is one
the pack already teaches somewhere (L2's pit, L3's patrol, L7's climb, L4's timing); what is new is
that one body has to make all of them inside one episode.

| what | measurement |
|---|---|
| Beatable in principle | `train-cli reach` — 6 platforms, all reachable |
| Not a lottery | **12 / 12 seeds solved**, first wins 30, 33, 54, 54, 58, 67, 68, 70, 81, 82, 90, 205 (medians and means below) |
| Harder than everything shipped | median first win **67.5** generations against the pack's slowest other median of **20** (L3, a v6 reading) |
| Not reachable by luck | `evo::tests::difficulty::no_earned_level_is_won_by_a_random_population` — 768 random genomes, no more than one winner, which is the pack's measured noise floor |

400 generations at pop 200 costs 0.03-0.04 s per generation on this machine, so a full run is about
fifteen seconds: the level is *slow for a population*, not expensive to search.

**Every number in this section is a re-measurement, because the ones it first carried did not
reproduce.** They were a **v7** reading (95 inputs, 1 723 genes) and the observation has since moved
to **v8**, which re-rolls every episode in every arm: run the old protocol on this build and seeds
1-3 come out at 105, 33 and 35 rather than the 39, 46 and 86 the table used to say, which is not a
regression but a different draw. What is quoted here is what this build does, and the arm that
matters is separable on it, because the trainer gained a second default in between — the trial rule
below, which judges a walker level on three patrol phases. `l9` has a walker in it.

| arm (v8, pop 200, 400 generations, `--stop-on-win`, 12 seeds) | solved | median | mean |
|---|---|---|---|
| shipped defaults (three patrol phases) | **12 / 12** | 67.5 | 74.3 |
| `--no-auto-trials` (one patrol phase, the pre-trial-rule protocol) | 10 / 12 | 76 | 106.7 |
| `--feedback 8` (the recurrent arm, shipped defaults) | **12 / 12** | **41** | **64.3** |

The trial rule is worth two seeds and about nine generations of median on the pack's only chain
level, which is the cost of judging a stochastic level on one phase — seeds 8 and 12 are *unsolved at
400* without it.

**And memory pays here, in generations.** Paired on the seeds, `--feedback 8` against memoryless:
**earlier on eight, later on four, nothing gained and nothing lost** (12 of 12 either way). The
median goes from 67.5 to 41 and the mean from 74.3 to 64.3, and the pairing is the shape the memory
section predicted: the two seeds that regress hardest (67 → 211, 30 → 98) are ones where carrying
sixteen extra genes costs more than the state buys, and the ones that gain most (205 → 42, 90 → 37,
58 → 31) are the runs where the body has to hold its nerve at the nineteen-tile patrol. On the
pack's other levels the same arm was neutral (the v6 memory table above), which is the honest form
of the claim: **memory is not a better policy, it is a longer one, and it pays exactly on the levels
that need a horizon.** It is still not the default; the first thing to reach for on a level with a
wait in it, yes.

### The trial rule: what it bought, and where it bought nothing

Item 3 (2.5) was wired this pass: three patrol phases where a level has walkers, one where it does
not, with `--no-auto-trials` releasing the rule so the one-phase protocol behind every earlier table
in this document is still reproducible. Twelve seeds, pop 200, 300-400 generations, `--stop-on-win`:

| level | three phases (shipped) | one phase | wall clock |
|---|---|---|---|
| `l3-gauntlet` | 12 / 12, mean 17.9 | 12 / 12, mean 17.6 | 6.7 s against 2.8 s |
| `l6-marathon` | 12 / 12, mean 9.8 | 12 / 12, mean 8.9 | 4.3 s against 1.6 s |
| `l9-wait-and-climb` | **11 / 12**, mean **136.8** | 9 / 12, mean 214.1 | 40.7 s against 26.2 s |

**On the two walker levels the pack shipped before this pass, the rule buys nothing.** Both levels
are won by the population in ten to twenty generations on every seed either way, and on `l6-marathon`
ten of the twelve first-win generations are *identical* between the arms (7, 5, 6, 6, 7, 4, 12, 10, 9,
12) because the run finds its winner before selection noise matters. The cost is not nothing: judging
a genome on three phases is 2.4x the wall clock for the same answer at the same population. What the
rule does not fix is also worth writing down, because it is the argument that was made for it —
`l6-marathon` still has its lottery (a mean first win near ten generations at pop 200 on the pack's
longest level is a level that a lucky agent walks), and no trial count repairs that, because the
problem is the level rather than the reading of it.

**On the level where the patrol phase is load-bearing, the rule earns its cost**: l9's nineteen-tile
patrol run is crossed in one direction or the other, so a genome judged on a single phase is judged
on a coin-flip of the level's one timed element. Three phases solve 11 of 12 seeds at a mean of 136.8
generations; one phase solves 9 of 12 at 214.1, gaining nothing and losing seeds 5, 9 and 11 — and on
the nine seeds both arms solve, the one-phase arm is later on six of them. 1.55x the wall clock for
36 % less search and two more solves is a trade worth making, which is the difference between this
level and the ones that came before it: **the rule matters in proportion to how much of the level is
a wait.**

### The clock: a time to beat, a reward for being quick, and a pressure towards new routes

The game could already tell you *whether* a level was solved and never *how fast*. Three things were
asked for, and they turned out to be one mechanism seen from three sides: show the time the AI took,
reward it for finishing quickly, and make it look for other ways to be quick. Fitness **v8** adds a
term (`SPEED_BONUS = 200`, paid only on a win, scaled by the level's own unspent frame budget), the
archive gained a finish-band axis, and every surface that reads a run now says the clock.

**The term is paid only on a win, and that is the whole design.** `Outcome::Stuck` and `Outcome::Died`
stop the clock too, so a reward for finishing quickly that paid anything for stopped episodes would be
a race to be killed — the park trap with the sign flipped. Two tests hold the shape of it:
`a_win_pays_for_the_time_it_did_not_spend` (half the budget unspent is half the bonus, a loss banks
nothing for dying early, any win still beats any loss by more than the bonus is worth) and
`a_quicker_win_cannot_be_bought_by_skipping_a_checkpoint` (100 frames saved buys 8.3 points against a
checkpoint's 16, so pace cannot be paid for by cutting a corner).

**The denominator is the level's own cap**, not a per-level number someone types in: `sim::frame_budget`
is `max(width, height) × FRAMES_PER_TILE`, which the sim *already* stops episodes at. Using it required
carrying it in `EpisodeStats` (`budget_frames`), which is why the sim bumped as well as the fitness
function — a score computed against a cap the episode cannot see is a score that can disagree with the
cap the episode was run under.

**Measured through the real trainer, against the same build with the bonus switched off** (which is
exactly fitness v7 — the term is additive and win-only). Pop 200, 60 generations, seed 1 and 2:

| level | seed | quickest win (200) | quickest win (0) | champion frames (200) | champion frames (0) |
|---|---|---|---|---|---|
| `l1-flat-run` | 1 | 6.15 s | 6.15 s | 373 | 416 |
| `l1-flat-run` | 2 | 6.15 s | 6.15 s | 371 | 504 |
| `l3-gauntlet` | 1 | 6.47 s | 7.13 s | 402 | 437 |
| `l3-gauntlet` | 2 | 6.40 s | 7.80 s | 385 | 493 |
| `l6-marathon` | 1 | 8.93 s | 8.93 s | 624 | 655 |
| `l6-marathon` | 2 | 8.67 s | 8.58 s | 530 | 658 |

The champion — the genome a run keeps and a viewer watches — is quicker or equal in five rows of six,
by up to 22%. The run's *record* is the honest half of the table: L1 hits the same 6.15 s either way
because 369 frames is the level's own floor for a body at that speed, and one row (L6 seed 2) has the
record ten frames worse with the bonus, which is what re-ranking the same population looks like. So the
claim this pass establishes is narrower than "it learns to be quicker": **among a generation of winners,
the clock decides which one is kept, and the kept one is quicker.**

**The pressure towards new routes is the archive, not the score.** A finishing band is now part of a
cell's key, so the grid went from 48 cells to 240 and two wins on the same route at different paces are
two niches rather than one incumbent fight. That is what lets a run keep a slow route *and* a quick one
for the same landing zone, so a faster solution to an old problem does not erase the only solution to
it — and the *speed coverage* a reader sees (`3 / 4 finish bands`) is the count that says a level has
been solved once rather than solved well. Coverage is reported over the 48 behavioural cells and not the
240 total; the difference matters because a run spread across a third of the ways of moving would read
as an eighth explored, and both front-ends were counting the wrong denominator before this pass
(`evo::ARCHIVE_CELLS` was 48 when that line was written and is 240 now).

**What the viewer got.** The run panel's readings went from three to four with the clock among them — it
is the one reading a person *acts* on, so it belongs on the panel rather than one press away in the
Details window. The playtest now runs a stopwatch (`Session::frames`, the sim's own tick count, never
the wall clock) and prints a race line against the run's record: `7.10s at the flag — the AI's best is
7.90s, 0.80s faster`, with `the same time, to the frame` for a tie and an explicit "the run has no win to
race yet" when there is nothing to beat. `--play` opens the window in it, because a scripted frame of
the playtest was the one view with no way to be rendered. The terminal's champion line, its training
summary (`clock 6.15s at the flag (gen 0 id 39, 85% of the level's 40.00s unspent, 4 of 4 finish bands
filled)` — the real L1 line below), the watch sign-off and a `*** QUICKEST WIN ***` note in the live view all read the same number
through `sim::format_time`, so 8.42 s means 8.42 s in all of them.

The two **look** findings the same pass fixed, both reported by the owner rather than found by the
suite: the pace control lived only inside the Details window, so the one setting a viewer changes
*while* watching was a keypress or a scavenger hunt — it is now a `− 8x +` stepper on the panel,
reporting one rung through the same `Command::Speed` the keys use. And the Details board was a column of
`ui.small(format!(...))` sentences, in two visually different styles across its four modes; it is now
label/reading rows (`ui::stat_row`, a fixed label column so a long label truncates instead of
overprinting its value) under the same section rule every board uses, with the level's own board
gaining the level's episode cap — the clock a solution is allowed to take, which is what the run's best
time is read against.

---

## 5. Graphics and assets

The art pipeline is exceptional — a generator, a manifest, palette shared by one
module, and tests that read the shipped PNG — and the last few findings (32–35) fixed
the real readability problems (player camouflage, invisible props, floating tree-line).
What remains is mostly *motion* and *feedback*:

1. **Juice is missing.** There is no audio at all, and no hit-stop. The researched
   ordering is: make controls responsive (already good — coyote time, jump buffer,
   corner behaviour are all in the physics bible), then add feedback per action —
   takeoff anticipation, landing dust (exists), a stronger impact for a stomp or
   death, a sparkle for the flag, and a short hit-stop on death/stomp only
   ([GameAnalytics juice](https://www.gameanalytics.com/blog/squeezing-more-juice-out-of-your-game-design)).
   **Camera shake is out of scope, decided against rather than deferred.** One was
   implemented — a landing, a stomp and a death each left a short impulse on the animator,
   which displaced the whole frame for up to two-tenths of a second — and it was removed
   on request because at this zoom a jolt that moves the level under the eye reads as the
   window being kicked rather than the body hitting the ground. The three events still
   read: each one spawns its own burst particles and squashes the body, and that is where
   the impact belongs. The frame the viewer chose is now exactly the frame they see, always.
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
5. **The grid is nearly full again:** 232 of 240 cells, after `art/ui.py` took it from 223 of 224 to
   16x15 (and grew by four more when the run's own transport joined it). The next *new family* widens it again — the generator refuses an atlas that overflows and the
   manifest docs say so, so the failure is an error rather than a missing tile — and the same is true
   of any new terrain material (three families × masks + blends). Worth knowing before the next
   request for "one more prop": the cheap moves are all reuse now.

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

* The clock pass: `cargo test --workspace` — **528 passed, 0 failed**;
  `cargo clippy --workspace --all-targets -- -D warnings` — **clean**. New tests: the two
  reward-shape tests in `evo::fitness` (the term's arithmetic and the checkpoint trade), a
  round-trip test that a written time is a clock (`sim::tests::a_time_in_frames_is_written_short_and_in_order`),
  the archive's speed-niche test and the run's-record test in `evo::ga`, the four-reading panel
  test, the pace-stepper ladder test, the playtest's stopwatch/race tests, and the champion-summary
  clock test. Ablation run through the real trainer (`SPEED_BONUS = 0.0` against `200.0`, five levels,
  two seeds, recipe in §4) rather than asserted. Frames rendered and inspected: `out/evolve-panel.png`
  (the clock reading and the `− 8x +` stepper on the panel), `out/play-race.png` (`--play`, the race
  line in the dock and `0.02s of 40.00s` in the status strip), `out/details-clock.png` and
  `out/details-level.png` (the two boards). The Details windows, the stepper and the race line were
  checked by eye on those frames — there is a display in this environment, but nobody at the keyboard
  to press the stepper, so its *wiring* is held by tests and its *look* by the frames.

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

* **This pass (the look of it, the art across the board, and what the sensors can see).**
  `cargo test --workspace` — **466 passed, 0 failed** (444 at the end of the last pass: +22,
  three for the window's own theme, six for the level shelf, five for the editor's shape row
  and two materials, four for the spawn rule in the sim and the editor, the search's archive
  and diversity readings, and the sensor re-pin). `cargo clippy --workspace --all-targets` —
  clean. The front end says what the window says: `--archive-children` in `TRAIN OPTIONS`, a
  `qd  N children bred from the archive, M of 48 niches filled` line after a run, the panel's
  own wording (`+N from the archive`, `diversity N/48 niches`) in the live search line, and
  two new CSV columns.
* **The art in `assets/` is the art in `tools/`, checked rather than assumed.**
  `python3 tools/gen_tileset.py` re-run against the shipped atlas reproduces
  `assets/tileset.png`, `assets/tileset.json` and `assets/tileset_preview.png`
  **byte-identically** (same md5), so the tileset in the repository is the tileset the
  generators draw and not a hand-edited copy of one. The atlas grew to ten rows: five
  materials — soil, sand, stone, brick and timber — each with its crown, body and coping
  bands, plus one seam band per ordered pair of families.
* **Every mode rendered and looked at**, through `evo-app --screenshot` on this machine: the
  title screen with its three doors, the editor with its palette grid, shape row and status
  bar, the evolve view with the search line, the fitness chart and the agent panel, and the
  champion replay with the population ghosted and the death marks on. Frames:
  `target/shots/shot-splash.png`, `shot-edit.png`, `shot-evolve.png`, `shot-replay.png`.
  The camera no longer takes the simulation's impulses at all: `SHAKE_FULL`, `App::shake`
  and `AgentAnim::shake` are gone, so the drawn camera *is* the viewer's camera rather than
  a copy displaced by the loudest lane (§5, item 1). Nothing in the suite could have caught
  a jolt — a shake is a per-frame displacement with no state a unit test can read — which is
  why the removal is structural: the only code that could move the frame was deleted rather
  than switched off.
* **The AI arm, measured on this machine** (i5-9400, six cores). Every number is
  `train-cli train` at pop 200, 12 seeds, 300 generations, `--stop-on-win`, with a run counted
  as solved at its first winning generation. The enemy-view bound is the change the pack
  actually feels, so it was swept three ways on `l3-gauntlet`, the level that exposed it:

  | enemy view distance | solved | mean first-win generation | wall clock, 12 runs |
  |---|---|---|---|
  | v4: unbounded (committed) | 12 / 12 | 25 | 3.8 s |
  | v5: four tiles | 12 / 12 | **41** | 15.2 s |
  | v6: eight tiles (shipped here) | 12 / 12 | **18** | 6.9 s |

  v5 fixed a real lie — a walker across the level read as a walker on top of you — and
  over-corrected: at four tiles the threat was only legible once it was inside the patch, and
  the population reached the flag later than it had with the dishonest vector. Eight tiles
  beats both. The levels without walkers cannot see it: between the two bounded arms,
  `scratch-tower` is 10 of 12 at a mean first-win generation of 94 in both and `l2-first-gap`,
  `l5-tightrope` and `l7-ascent` are 12 of 12 at generations 2, 16 and 10 in both. (The tower's
  own movement is the *scoring* change of the same pass — directional checkpoint credit — and
  it is a gain: 94 against the committed build's 122 at the same 10 of 12.) The sim's own
  throughput is unchanged: 60 000
  ticks in ~3 ms in both builds, generation-1 cost on `scratch-tower` within a millisecond.
  On the levels with walkers the random population lives longer under the honest observation
  (generation 1 at pop 200: 21 ms at HEAD, 51 ms here), so the *per-generation* wall clock on
  `l3-gauntlet` and `l6-marathon` is ~2.4x HEAD's while episodes are longer — that is the
  price of the change, and it is paid against a wall clock to solve that is the same order of
  magnitude (6.9 s for twelve seeds against 3.8 s) rather than a slower search.
* **The archive trickle, re-measured at v6 and left off.** `scratch-tower` at 12 seeds: 10 of
  12 solved at a mean first-win generation of 94 with the trickle off, 7 of 12 at 121 with two
  archive children, and 6 of 12 at 88 with four — the dose that loses the hardest level in the
  pack, reproducing the reading that turned the mechanism off. `l3-gauntlet` is where the same
  arm used to win (15 against 41, measured at v5); at v6 it is 16 against 18, i.e. inside the
  noise of twelve seeds, because the honest observation already bought what the trickle was
  papering over. `l6-marathon` solves in single-digit generations either way (9 against 11).
  So the knob stays a knob, and the panel's diversity reading stays honest with it at 0.

* **This pass (the roadmap's own order: the baseline first, then the trial rule, the archive gate
  and a level that is a chain).** `cargo test --workspace` — **480 passed, 0 failed** (470 at the
  end of the last pass: +10 = the ES arm's budget test, its two command-line tests, the rewritten
  trial-averaging test, the enemy-direction slot and its layout pin, the new level's row in the
  pack table and in the difficulty ladder). `cargo clippy --workspace --all-targets` — clean. The
  four measurements are in §4 and each is a first-win-generation table: the **ES baseline**
  (`train-cli es`, 0 of 12 on the tower against the GA's 10 and 2 of 12 on the gauntlet against
  12), the **archive gate** (9 of 12 at a mean of 76 on the plateau, but 1 of 12 on the hazard
  floor where the ungated trickle earns 3), the **trial rule** (nothing on the two walker levels
  the pack shipped before this pass, 11 of 12 against 9 and a third less search on the level whose
  patrol run is load-bearing) and **`l9-wait-and-climb`** (a median first win of 160 generations,
  against the pack's old slowest median of 20, with `--feedback 8` cutting the mean by a third).
* **Two tests changed because a measurement moved under them, and both changes are measurements
  rather than loosened assertions.** The window's and the terminal's curriculum tests need a
  sixty-generation cap because at pop 2 on `l5-tightrope` this build's last new best is generation
  21 and the 25-generation stall window the panel uses therefore opens at 46 — a forty-generation
  cap ended the run before the stall the tests exist to exercise (sensors v7 moved the
  trajectory). And `the_search_bookkeeping_is_as_reproducible_as_the_rest_of_the_run` compares a
  self-tuning run's sigma path against a fixed one over **forty** generations rather than twelve,
  because v7's extra input put this fixture's first twelve generations at a success rate the
  one-fifth rule does not act on — so the comparison was two constant 0.30 paths, failing for
  exactly the reason the assertion exists. Neither fix weakens a claim: both keep the claim and
  give the mechanism the room to act.
* **This pass (the window's furniture: a dock, and one vocabulary for the things you press).**
  `cargo test --workspace` — **512 passed, 0 failed** (480 at the end of the last pass; six of the
  additions are `app::ui`'s own and the rest landed with the v8 sensor slot, the trial rule's
  defaults and the chain level's row in the same uncommitted pass as this one). `cargo clippy
  --workspace --all-targets -- -D warnings` — **clean**. The six are the press sink and the inverted
  edges, the five distinct faces, the longest word in the switch measured against the room its cell
  has for it, the banner declining to draw into too little room, the widgets really painting (counted
  in *vertices* in a headless context, which is the only thing that separates "drew a chunky plate"
  from "allocated space and painted nothing"), and a button with a misnamed icon still being a
  button. §3 has the mechanisms.
* **The dock was rendered and *looked at*, and the first two renders are why that mattered.**
  `evo-app --screenshot` again: `target/shots/shot-dock-edit.png` and `shot-dock-evolve.png`.
  Reading them found two defects no test in this suite could see. The plaque's two lines were laid
  out **side by side** — a `Frame` inside a horizontal bar inherits the bar's layout, so the sign was
  235 points wide instead of 140 and cost the bar a control's worth of room. And the Evolve reading
  was missing entirely: the banner measures itself against the room left after the file path and had
  quietly declined. Both are fixed (the plaque forces a top-down layout, and the path is gone from
  the dock because the plaque already names the file), and the frames in `target/shots/` are the
  evidence rather than the intent — with `--screenshot` and no display in this environment, a frame
  is the only instrument there is for this half of the work.
* **Every champion file on disk before this pass is now refused**, for the documented reason: the
  sensor version moved to v7 and the network's input count with it (95 to 96 inputs, 1 723 to
  1 739 genes), which is what `champions/README.md` says those files are (a build artifact of a
  version trio, never committed). Local files were not regenerated here beyond the measurement
  runs; the next `train` writes fresh ones.
* **This pass (the look, second pass: fewer words, one window, and a theme that was never
  installed).** `cargo test --workspace --no-fail-fast` — **514 passed, 0 failed** (512 at the end of
  the last pass; the two additions are the theme-slot regression test and the guard that every glyph
  the window draws is in the atlas — the switch's width test *replaced* the clearance one rather than
  joining it, and the UV test is the same test, failing on a new grid shape). `cargo clippy
  --workspace --all-targets -- -D warnings` — **clean**. Two of the tests that moved were failures
  first, and they are the point of the pass:
  * `app::atlas::tests::every_shipped_sprite_maps_inside_the_image_at_one_cell` failed the moment the
    atlas grew a fifteenth row — `14 * (1/15) + 1/15` is `1.0000001` in f32, so the bottom row's far
    edge landed a hair outside the texture. That is a real (if invisible) sampling bug in
    `Atlas::uv` that the *old* grid happened to hide, and the fix is to divide cell coordinates by
    the grid rather than multiply them by a reciprocal, which puts the last cell's edge at exactly
    1.0. Caught by a test written two passes ago for a different reason.
  * `app::theme::tests::a_theme_switch_cannot_repaint_the_window` is new, and it is the regression test
    for the details window drawing itself in egui's stock dark colours inside a cream window. It
    asserts the theme in **both** of egui's style slots, because the broken one is the slot nothing on
    screen is using yet — `ctx.style()` had been passing all along.
* **Frames rendered and looked at, again, and again for a reason.** `--screenshot` at 1360x800 for the
  editor, the replay and the evolve view, and one deliberately at the 900-point minimum width with the
  pack's longest level name (`target/shots/s2-edit.png`, `s2-play.png`, `s3-evolve.png`,
  `s3-narrow.png`, and `s3-details.png` with the level-details window open, which the constructor's
  `details_open` was flipped for on the command line and flipped back). The dark details window is the
  one defect four of those renders had been hiding in plain sight since the *previous* pass: every
  panel that looked right was a frame with an explicit fill, so nothing egui styled itself had ever
  been in a frame anyone read. `/tmp`-style throwaway HTML pages that inline the PNGs as data URIs
  (`tools/art/shot_viewer.py`) were the only way to see them here — no display, no browser.

**Not verified:** the pointer feel of finding 1.6's ghost, and any interactive
clicking at all — there is no display in this environment, so every app check goes
through `--screenshot` and the logic-level tests. The tile the mouse reports is
`Camera::to_tile`'s, which `the_flag_is_drawn_in_the_cell_the_eraser_addresses` now
exercises against a real frame, but a human at the window is still the only thing that
can confirm the drag reads right. The theme, the shelf, the splash's three doors and the
editor's shape row are in the same position: they are held by their own tests and by
rendered frames, and whether the result *looks* right is the one claim a screenshot can
only support rather than settle.

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
| The window's palette is installed, and its four surfaces are four depths | `app::theme::tests::apply_installs_a_theme_and_the_surfaces_are_four_distinct_depths` |
| Installing the theme twice changes nothing | `app::theme::tests::applying_twice_changes_nothing` |
| Every material has a thumbnail colour, and no two share one | `app::theme::tests::every_material_has_a_thumbnail_colour` |
| The shelf lists every shipped level and each one opens | `app::shelf::tests::the_shelf_lists_every_shipped_level_and_each_one_loads` |
| A card names the level's own materials and size | `app::shelf::tests::a_card_reports_the_levels_own_materials_and_size` |
| A broken level is a card that says why | `app::shelf::tests::a_broken_level_is_a_card_that_says_why` |
| A thumbnail draws the floor and not the sky | `app::shelf::tests::the_thumbnail_draws_the_floor_and_not_the_sky` |
| A long level is downsampled into the thumbnail's grid | `app::shelf::tests::a_long_level_is_downsampled_to_the_thumbnails_grid` |
| Brick and timber are paints of their own | `app::editor::tests::the_brick_and_timber_brushes_paint_their_own_tiles` |
| A rectangle drag fills the box and is one undo step | `app::editor::tests::a_rect_drag_fills_the_box_and_is_one_undo_step` |
| A line drag leaves a connected run | `app::editor::tests::a_line_drag_leaves_a_connected_run` |
| The spawn refuses a cell with nothing under it | `app::editor::tests::the_spawn_refuses_a_cell_with_no_ground_under_it`, `sim::level::editing_tests::a_spawn_needs_an_empty_cell_with_ground_under_it` |
| A fresh document starts on ground | `app::editor::tests::a_fresh_document_starts_on_ground` |
| A walker is out of sight beyond the view distance and legible inside it | `sim::tests::levels::an_off_grid_or_distant_walker_is_out_of_sight_until_approached` |
| A sighted walker's direction is observed, and follows it as it turns | `sim::tests::sensors::the_enemy_direction_slot_follows_the_walker_as_it_turns` |
| The observation layout is what it says it is, at 96 inputs and version 7 | `sim::tests::sensors::the_vector_has_the_documented_named_layout` |
| The whole observation vector is pinned again, for v7 | `sim::tests::sensors::golden_observation_hash_at_a_fixed_state` |
| The network's width follows the sim's table, at 96 inputs and 1 739 genes | `evo::genome::tests::default_shape_matches_the_plan` |
| Every trial selection runs is really run, on its own seed, and their mean is the score | `evo::ga::tests::every_selection_trial_is_run_on_its_own_seed_and_averaged_into_the_score` |
| The walker rule is three patrol phases, and `--no-auto-trials` releases it | `evo::ga::tests::a_walker_level_is_judged_on_more_than_one_patrol_phase` |
| The baseline's episode budget is the GA's, or it is not a baseline | `evo::es::tests::the_baselines_default_budget_matches_the_gas` |
| The `es` command's defaults come from its trainer, and its budget from the GA's | `train_cli::commands::tests::the_es_command_lines_defaults_are_the_trainers_defaults_and_the_gas_budget` |
| The `es` subcommand runs a shipped level end to end, and refuses `train`'s flags | `train_cli::commands::tests::the_es_subcommand_runs_on_a_shipped_level` |
| The archive gate is off by default, and the flag is the only way to it | `evo::ga::tests::the_archive_can_wait_for_a_run_that_has_stopped_having_ideas` |
| The pack's chain level is in the round trip, the shape table and the difficulty ladder | `sim::level::editing_tests::every_shipped_level_round_trips_through_the_writer_with_the_same_hash`, `sim::tests::levels::all_shipped_levels_load_with_the_documented_shape`, `evo::tests::difficulty::no_earned_level_is_won_by_a_random_population` |
| The observation layout did not move when the enemy range did, at version 6 | `sim::tests::sensors::the_vector_has_the_documented_named_layout` |
| The whole observation vector is pinned again, for v6, with a walker in the frame | `sim::tests::sensors::golden_observation_hash_at_a_fixed_state` |
| The archive breeds only when it is asked to, and fills either way | `evo::ga::tests::the_archive_breeds_only_when_it_is_asked_to` |
| Every child bred from the archive is counted, and the knob turns them off | `evo::ga::tests::archive_children_are_bred_every_generation_and_the_knob_turns_them_off` |
| The archive keeps the best genome per niche and breeds from its own incumbents | `evo::ga::tests::map_elites_archive_retains_best_genome_per_behavioural_niche`, `evo::ga::tests::the_archive_breads_from_its_own_incumbents_and_moves_on` |
| Pressing a plate sinks its content by `PRESS_SINK`, and a dead plate does not sink | `app::ui::tests::a_pressed_plate_sinks_its_content_and_a_resting_one_does_not` |
| A pressed plate is inset: its lit and shaded edges swap | `app::ui::tests::a_pressed_plate_inverts_its_edges` |
| Every plate state has its own face, and the chosen one is the accent with dark type | `app::ui::tests::every_state_has_its_own_face` |
| The switch's chosen cell grows to hold the word it shows, word by word | `app::ui::tests::the_chosen_cell_has_room_for_the_word_it_shows` |
| A theme switch cannot repaint the window (both of egui's slots are this game's) | `app::theme::tests::a_theme_switch_cannot_repaint_the_window` |
| Every glyph the window's own controls draw is in the shipped atlas | `app::atlas::tests::every_glyph_the_window_draws_is_in_the_atlas` |
| The controls paint, and a disabled one answers no pointer | `app::ui::tests::the_controls_paint_and_the_switch_reports_the_cell_under_the_pointer` |
| A button whose icon is missing is still a button | `app::ui::tests::a_button_without_a_working_icon_still_draws_its_label` |
| A click on a door enters it, and the press does not leave first | `app::splash::tests::a_click_on_a_door_enters_it_and_the_press_does_not_leave_first` |
| The doors arrive with the caption and sit under the world | `app::splash::tests::the_doors_arrive_with_the_caption_and_sit_under_the_world` |
| The editor draws a document on a surface, and the sky's hills stay out of it | `app::view::tests::the_sky_backdrop_draws_clouds_over_the_same_level` |
| Every shipped sprite maps inside the texture at exactly one cell, edge included | `app::atlas::tests::every_shipped_sprite_maps_inside_the_image_at_one_cell` |
| A win pays for the budget it did not spend, and only a win does | `evo::fitness::tests::a_win_pays_for_the_time_it_did_not_spend` |
| The clock a real episode is paid on is the sim's own frame count against the level's own budget | `evo::fitness::tests::the_clock_an_episode_is_scored_on_is_the_sims_own` |
| A quicker win cannot be bought by skipping a checkpoint | `evo::fitness::tests::a_quicker_win_cannot_be_bought_by_skipping_a_checkpoint` |
| A time in frames is written one way, in both shapes, and reads back as the frames it was made from | `sim::tests::a_time_in_frames_is_written_short_and_in_order` |
| The archive keeps a niche per speed and still counts one way of moving | `evo::ga::tests::the_archive_keeps_a_niche_per_speed_and_counts_one_behaviour` |
| A run reports the quickest win it has recorded, and keeps it across a generation that wins nothing | `evo::ga::tests::a_run_reports_the_quickest_win_it_has_recorded` |
| The run's panel says four things, including the clock, and an em dash when there is no win to time | `app::panel::tests::the_run_panel_says_four_things` |
| The panel's pace stepper moves one rung, stops at both ends, and reports rather than applies | `app::panel::tests::the_panel_has_a_pace_stepper_that_walks_the_same_ladder_as_the_keys` |
| The playtest's stopwatch is the sim's frame count, not the frames drawn | `app::play::tests::the_stopwatch_is_the_sims_frame_count_and_not_the_frames_drawn` |
| The race line says the time, where it got to, and how it stands against the AI's best | `app::play::tests::the_race_line_says_the_time_and_how_it_stands_against_the_ai` |
| A session remembers its quickest win and keeps it across a restart | `app::play::tests::a_session_remembers_its_quickest_win_and_keeps_it_across_a_restart` |
| Being in front of the AI is the clock against the clock, running or finished | `app::play::tests::being_in_front_of_the_record_is_the_clock_against_the_clock` |
| A champion's summary line says its clock as a time as well as a frame count | `evo::champion::tests::a_champions_summary_says_its_clock_as_a_time_and_a_frame_count` |
