# Evolution Platformer

Build a 2D platformer level, then watch a population of neural networks evolve to
beat it. The entertainment product is the **visible learning arc** — stands still,
walks, jumps, beats the level — not the algorithm.

The game and the training live in the same window: draw a level with the mouse, press
play and run it yourself, then hand it to a few hundred random networks and watch the
best ones get better. A terminal driver does the same training headlessly.

A run is a **stopwatch as well as a score**. The panel shows the quickest win the
population has found, the playtest runs the same clock so you can try to beat it, and the
reward for finishing quickly is part of what the search is selected on — so "solved" is a
level, and "solved in 6.15 s of the 40 s it was allowed" is a run.

This repository implements [development_plan.md](development_plan.md), the design
document. **This README is what the thing is and how to run it.** The measurements, the
numbered findings, the verification detail and the deferred work are in
[FINDINGS.md](FINDINGS.md), and the art pass with its remaining rough edges is in
[REVIEW.md](REVIEW.md).

## Status

| Milestone | State |
|---|---|
| **M0** scaffolding | done |
| **M1** sim + hand-written levels | done |
| **M2** GA + headless training + reachability oracle | done |
| **M3** watch mode + fun gate | Tiers 1 and 2 done — **the fun gate is the owner's call** |
| **M4** editor + content | pixel renderer, generated art, drag-and-drop editor with a browsable level shelf and a whole-document mirror, playtest, and the whole watch mode inside the window. Not yet: a curated starter pack |
| **M5** web/release | not started — except its stall chain, which was done early |

Version trio: **sim 0.9.0 / sensors v7 / fitness v8**. Every champion file records all
three and is *refused* rather than replayed against different physics or a different
objective. What each version changed, and why, is in
[FINDINGS.md](FINDINGS.md#the-version-trio).

**That trio divides the work into two kinds, and it is worth knowing which kind you are
doing before you start.** A change to the physics, the sensors or the fitness moves a
version, which refuses every champion trained under the old one — the `champions/`
directory is a compatibility surface, and retraining the pack is the bill. A change to the
window is free: the editor, the panels, the art and the keys cannot invalidate a champion,
because no champion is a function of them. Nothing about the arithmetic of the sim is
different for that — it is the same `cargo test --workspace` either way — so the only
thing at stake is whether the artifact on disk still counts.

## Quick start

```bash
cargo test --workspace                 # 553 tests
cargo build --release

# The game: draw a level, drag tiles out of a palette, play it yourself.
python3 tools/gen_tileset.py           # only if you change the art
cargo run --release -p app             # title screen, then l1-flat-run.json
#   any key or click leaves the title screen for the editor
#   drag a palette item onto the level to place it; click to paint
#   right-drag erases, middle-drag (or alt-drag) pans, wheel zooms
#   ctrl+click takes the brush from the tile under the pointer; shift-drag draws a line
#   ctrl+Z / ctrl+Y undo   G grid   F fit   ctrl+S save   P play/pause-mode
#   Mirror (in the tool bar) flips the whole level left to right, in one undo step
#   in play: arrows or W/A/S/D move and jump, R restarts, P or Esc returns
#   ? (or F1) opens the key card for whichever view is on screen
cargo run --release -p app -- --level levels/l6-marathon.json

# Train on the level you are editing, and watch it happen. The panel reads how far the
# population has got, how many reached the flag, **the quickest win so far** and the best
# fitness; the pace it plays at is the − 8x + control on the panel (or - / +), which is
# the one setting you change while watching.
cargo run --release -p app -- --evolve --level levels/l2-first-gap.json
#   space pause   n one generation   -/+ speed   m/M mutation rate
#   left/right scrub the recorded champions   h back to live
#   s save the best-ever champion   f follow the leader / whole level   v the run's replay

# Try to beat the AI. The playtest runs a stopwatch against the run's quickest win, in
# the line at the top of the window: "7.10s at the flag — the AI's best time is 7.90s,
# 0.80s faster". The clock waits for your first key, a death restarts itself after a
# second, and G (or the Ghost button) shows the record-holder running beside you.
# Train first (or pass --evolve) if the run has no win to race yet.
cargo run --release -p app -- --play --level levels/l2-first-gap.json
cargo run --release -p app -- --play --screenshot out/play-race.png

# Watch the run's answer rather than its search: the best agent it found, playing at
# the speed the game runs at, with the population it last scored ghosted beside it.
cargo run --release -p app -- --replay --generations 20 --level levels/l3-gauntlet.json
#   space pause   R run it again   G ghosts   +/- speed   F follow   V or Esc back

# Screenshots, for the renderer without a human: a frame, the title screen, a replay.
cargo run --release -p app -- --screenshot out/frame.png
cargo run --release -p app -- --splash --screenshot out/splash.png
cargo run --release -p app -- --replay --generations 20 --no-ghosts \
    --screenshot out/replay.png
cargo run --release -p app -- --evolve --generations 20 --screenshot out/evolve.png

# A run can fall back to an easier level when it stalls, carry the population it grew
# there back to the hard one, and resume from the checkpoint that step writes.
cargo run --release -p app -- --evolve --level levels/l5-tightrope.json \
    --curriculum levels/l3-gauntlet.json
cargo run --release -p app -- --evolve --level levels/l5-tightrope.json \
    --resume champions/l5-tightrope.population.json

# With no authored fallback the run derives its own (findings 50–51): the same level
# from the furthest spot its best genome has actually stood, with the flag moved to
# the stand the route to the flag reaches next — one crossing checked against the jump
# oracle, not guessed from straight-line distance. Each detour is then one move —
# judged on the move and the climb together, so nothing is forgotten — and it steps
# back again whenever it stalls at a new spot; the derived level is written to
# <out>/<level>.practice.json. Each detour also starts seeded with jump-timing takes
# on the hop (finding 53): one to three of the stuck genome's jump-decision genes
# moved, in the population's weakest slots. Measured flat on finding 51's protocol —
# the detour already learns a hop in a median of two generations when it learns it —
# so the seeds are documented rather than advertised.
cargo run --release -p app -- --evolve --level levels/l5-tightrope.json
```

The terminal front-end takes the same run flags with the same meanings, so a run can
be started in one front-end and resumed in the other:

```bash
./target/release/train-cli reach --all --levels-dir levels --verbose   # beatable at all?
./target/release/train-cli watch --level levels/l5-tightrope.json      # watch it learn
./target/release/train-cli train --level levels/l3-gauntlet.json \
    --generations 200 --pop 200 --stop-on-win --csv out/l3.csv         # headless
./target/release/train-cli replay --champion champions/l3-gauntlet.json
./target/release/train-cli play --level levels/l1-flat-run.json        # play it yourself
./target/release/train-cli render --level levels/l3-gauntlet.json --steps 400 --stride 40
./target/release/train-cli sensors      # the observation layout and the physics bible

# One policy, several levels: `--suite` scores every generation on all of them and ranks a
# genome by the level it is *worst* at, and `score` asks a saved champion the same question.
./target/release/train-cli train --suite levels/l2-first-gap.json,levels/l5-tightrope.json,levels/l9-wait-and-climb.json \
    --generations 300 --pop 200 --stop-on-win
./target/release/train-cli score --champion champions/l2-first-gap-suite.json \
    --suite levels/l2-first-gap.json,levels/l5-tightrope.json,levels/l9-wait-and-climb.json
```

`train-cli --help` lists every option. The window takes the same ones as its own
commands (`--evolve`, `--play`, `--replay`, `--screenshot`, `--splash`). `train` and
`watch` both print the run's quickest win when they finish —
`clock   6.15s at the flag (gen 0 id 39, 85% of the level's 40.00s unspent, 4 of 4 finish
bands filled)` — and every champion line writes its episode's clock as a time as well as
a frame count.

## Levels

`levels/` holds the pack. Each level teaches one thing and the next one assumes it:

| Level | What it is for |
|---|---|
| `l1-flat-run` | nothing at all — the smoke test, won by a random population at generation 0 |
| `l2-first-gap` | the first pit, and that a jump has to be *decided* |
| `l3-gauntlet` | the pack's slowest start: pits and spikes in sequence, no free route |
| `l4-rhythm` | a jump that has to be timed rather than merely made |
| `l5-tightrope` | narrow ground and spikes: landing precision |
| `l6-marathon` | endurance — a long floor, and the frame budget to match |
| `l7-ascent` | the wall jump, as a route up rather than around |
| `l7-wall-school` | the same move with nothing else in it, for when the ascent is too much |
| `l8-long-jump` | sprint, and the only gap a running jump cannot clear |
| `l9-wait-and-climb` | a *chain*: a spike, a pit, a timed patrol, a hazard patch, a step, a wall climb, a ledge gap, a spike and a climb, with the spawn at column 2 and the flag at column 56 |

The generation each level falls at, per seed, is measured in
[FINDINGS.md](FINDINGS.md#difficulty-the-generation-the-population-first-wins) — nine
levels, twelve seeds, pop 200 — along with what a solve costs to *watch*.

`levels/scratch-*.json` are working levels rather than part of the ladder;
`scratch-tower.json` is a 10×22 climb whose flag is at the **top**, an example of a
vertical level (see [the level format](#level-format)).

## Level format

Human-writable ASCII grid plus an entity list:

```jsonc
{
  "format": "evo-platformer-level",
  "level_format_version": 1,
  "name": "L2-first-gap",
  "width": 60, "height": 15,
  "spawn": {"x": 2, "y": 12},
  "tiles": [
    "............................................................",
    // one string per row, top row first
  ],
  "entities": [
    {"type": "walker", "x": 30, "y": 12},
    {"type": "goal", "x": 57, "y": 12}
  ]
}
```

Tiles: `.` empty, `#` solid, `S` sand (solid exactly as `#`, and a different material
for the *view* only), `^` spike (lethal, not solid), `G` goal (normalized into a goal
entity at load time).

**A level has exactly one flag**, declared as a `goal` entity or as a `G` tile; a file
with two, or with none, is refused at load with the reason. The editor keeps the same
rule: the goal brush *moves* the flag rather than adding a second one, and the eraser
takes it away altogether — including the flag a new level starts with, which is yours
to delete and put down somewhere else. A document with no flag can be drawn but not
saved, and the status line and the oracle's verdict both say so.

**Coordinates are tile coordinates, and an entity at `(x, y)` stands on the *bottom*
edge of row `y`.** An entity at `y = 12` resting on ground tiles in row `13` is written
`y = 12`. Out-of-bounds tiles are solid to the left and right (invisible level walls)
and empty above and below (you may jump off the top; falling out of the bottom is a pit
death).

**A level may be as tall as it likes and the goal may be anywhere.** The objective is
the distance to the flag, not progress to the right, so a level can climb, descend or
run sideways — the frame budget is set by the level's *longer* side, and a flag above
the spawn is answered by the same sensors, fitness and camera as one to the right.
(This was not always true; see
[FINDINGS.md](FINDINGS.md#the-version-trio) for what a rightward-only objective did to
the first tower.) Check any new level with the reachability oracle before training on
it:

```bash
./target/release/train-cli reach --level levels/my-level.json --verbose
```

It says whether the flag is reachable at all, which jump cannot be made if not, and
which ability the level depends on — disabled-sprint and disabled-wall-climb runs are
how the pack keeps `l7-ascent`, `l7-wall-school` and `l8-long-jump` honest about
teaching what they are named for.

## Physics bible

All constants live in `sim::PHYSICS` and the jump capabilities are *derived* from them,
never tuned separately:

| Quantity | Value |
|---|---|
| `gravity` | 0.0125 tiles/tick² |
| `jump_v0` | 0.25 tiles/tick |
| `run_speed` | 0.10 tiles/tick (6 tiles/s) |
| `sprint_speed` | 0.15 tiles/tick (9 tiles/s), ramped at 0.005/tick² from `run_speed` |
| `max_fall` | 0.75 tiles/tick |
| jump height | `v0²/(2g)` = 2.50 tiles continuous, **2.375 measured** |
| air time | `2·v0/g` = 40 ticks |
| jump distance | `run_speed · air_time` = 4.00 tiles |
| sprint jump distance | `sprint_speed · air_time` = 6.00 tiles |
| sprint runway | `v²/(2a)` = 0.5 tiles to `run_speed`, **1.125 to `sprint_speed`** |
| grounded turn | `run_accel × turn_accel_scale` = 0.025 tiles/tick² — 8 ticks from `run_speed`, 12 from sprint. **Grounded only**: the airborne turn stays at `run_accel × air_accel_scale`, because that is what keeps the pack's pits lethal (see FINDINGS.md) |
| wall slide | `wall_slide_fall` = 0.06 tiles/tick (a tenth of `max_fall`) |
| wall jump | `wall_jump_v0` = `jump_v0`, so 2.50 tiles — one jump's worth — plus a push away from the wall |
| shortest possible jump | 1.155 tiles (`min_jump_hold` 4 ticks — one decision block — then `jump_cut`) |
| jump ladder | one rung per decision block of hold: 1.155 / 1.70 / 2.085 / 2.31 / 2.375 tiles |
| forward hop, braked | 0.90 tiles (one block of hold, then `brake`) — the measure of how finely a landing can be aimed |

The discrete Euler integration in the sim peaks slightly lower than the continuous
formula; `PHYSICS::discrete_max_jump_height()` returns the real number, and it is what
the reachability diagnostics quote. Three rules matter as much as the numbers:

* **A jump needs a fresh press.** Holding the button jumps once; landing with it still
  held does not jump again. Presses are *buffered* for 6 ticks, so a press a few ticks
  before touchdown still fires — deliberate jumping is forgiving, mashing is not.
* **A wall jump is automatic.** Press into a wall while airborne and the body clings
  and slides; press *jump* there and it launches away from the wall, with no separate
  button, because the verb does not change. The cling is a pressed state: let go of the
  direction and the slide ends on the next tick. This is the move `l7-ascent` is built
  out of.
* **A hop's length is a knob, not a constant.** The agent picks a hold *duration* in
  blocks and nothing else, so airtime — and therefore carry — follows from the hold;
  and `brake` bites in the air as well as on the ground, which is what lets a body kill
  its own momentum mid-flight and land somewhere no coasted hop can reach.
* **Changing your mind is cheap on the ground and expensive in the air.** A grounded
  reversal accelerates at 2.5× the ordinary rate, so the turn a player feels on flat
  ground is deliberate rather than a drift; the airborne turn is deliberately left slow.
  A body that walks off a pit lip at running speed must not be able to press back onto
  the pit wall, and that is exactly what makes the pack's pits fatal.

## Development

```bash
cargo test --workspace                  # 553 tests: sim, evo, render, train-cli, app
cargo clippy --workspace --all-targets  # clean
python3 tools/gen_tileset.py            # regenerate assets/ from tools/art/
```

```
crates/sim/        the deterministic core: tile grid, AABB physics, sensors, oracle
crates/evo/        fixed-topology genome, GA, fitness, champion files
crates/render/     read-only views: ASCII frame/scene, keyboard play, text charts
crates/train-cli/  headless driver: train, watch, reach, replay, play, render, sensors
crates/app/        the game: pixel renderer, drag-and-drop level editor, playtest
levels/            hand-written *.json levels
assets/            generated pixel art: the tileset atlas, its manifest, a preview
tools/art/         the art itself: terrain, characters, props, one module each
champions/         generated champion files (git-ignored)
```

Where to read next: [development_plan.md](development_plan.md) for the design,
[FINDINGS.md](FINDINGS.md) for what was measured and what deviated and why (findings
are numbered and append-only, and source comments cite them by number), and
[REVIEW.md](REVIEW.md) for the art pass and the outstanding rough edges.
