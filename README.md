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
| **M4** editor + content | pixel renderer, generated art, drag-and-drop editor with a browsable level shelf and a whole-document mirror, playtest, the two **trap tiles** (brittle ground and spring pads), and the whole watch mode inside the window. Not yet: a curated starter pack |
| **M5** web/release | not started — except its stall chain, which was done early |

Version trio: **sim 0.11.0 / sensors v9 / fitness v8**. Every champion file records all
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
cargo test --workspace                 # 563 tests
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

# Where a run starts. `--resume` continues a **population** checkpoint (the artifact a stall
# writes); `--from-champion` starts a new population around one saved **genome** — the champion
# in slot 0 and a shell of reheated copies of it behind. The second is what to reach for when a
# run was stopped without a checkpoint, when the level has been edited since, or when one
# level's policy should face a harder one: the champion's own level is *not* required to match
# (that mismatch is printed, not refused), and a seeded run writes `<level>-seeded.json` rather
# than overwrite the file it started from.
./target/release/train-cli train --level levels/l5-tightrope.json \
    --from-champion champions/l8-long-jump.json --generations 200 --stop-on-win

# Where a run starts, on a level nobody has trained on yet. `--from-route` walks **this level's
# own route** (the oracle's graph, driven through the real episode loop), clones the walk into a
# genome and seeds a quarter of generation 0 from it — so a level you have just drawn is not a
# level the search has to rediscover. Measured: two to four times fewer generations to a first
# win, and a starting policy at the 90th-100th percentile of a random population's fitness.
# It is a walker and not a solver, and the reason is the decision cadence: the oracle presses a
# take-off on the tick its script reaches it, while a policy decides once every four ticks — so
# the clone (which reproduces every one of the walk's decisions at a 4000-epoch fit) still walks
# into the pit. At a one-tick cadence it solves l2, l4 and l5 on every seed, which is the pinned
# measurement. The run prints the replay's own verdict before it starts, and `--warm` moves how
# much of the population it fills.
./target/release/train-cli train --level levels/my-new-level.json \
    --from-route --generations 200 --stop-on-win

# A seed sweep as one command, and the paired test that reads two of them. A **protocol** is a
# named (level, pop, generations, seeds) row — `--list` prints them — and an **arm** is one run
# of it with whatever flags you pass. The arm is recorded to bench/<protocol>-<arm>.json
# together with every knob that was in force, so a table in the notes can be reproduced a year
# later; `--compare` reads two arms back seed by seed, with the exact sign test over the seeds
# they disagree on.
./target/release/train-cli bench --list
./target/release/train-cli bench --protocol chain --arm default
./target/release/train-cli bench --protocol chain --arm hidden32 --hidden 32
./target/release/train-cli bench --compare bench/chain-default.json,bench/chain-hidden32.json
```

Two knobs are worth knowing before reading a table, because they change what the search can be
rather than how it steps. `--hidden 32` (or `--hidden 32,16`) widens the net — the shipped recipe
is one layer of 16, 1 739 genes, and the run's header prints the net it is actually training.
`--rank-pessimism 0.5` changes what "best" means without changing anything that is recorded: the
ranking sits between the mean of a genome's own trials (0.0, the default) and the worst of them
(1.0), which is the knob that answers finding 14 — an elite re-rolled out of the run by a patrol
phase it never saw. Both are measured in [FINDINGS.md](FINDINGS.md#what-the-search-can-be-and-what-the-stall-chain-is-worth).

A third pair is about *which episodes* a generation is judged on, and it is now the default rather
than a knob to reach for. Every genome of a generation is played on the **same** episode
(`common_episodes`), so a tournament compares policies instead of two draws of the patrol phase —
and because the episodes are paired, the extra trials can be spent only where they still matter
(`race_episodes`): trial 0 for everyone, trial 1 for the better half, trial 2 for the better quarter.
That is 1.75 episodes per genome on a walker level instead of 3.00, and 29 % off the wall clock at
pop 200 on `l3-gauntlet` for the same first-win counts. `--no-common-episodes` restores the
per-genome policy every table in [REVIEW.md](REVIEW.md) taken before it was measured under (and
turns racing off, since a race cannot be sounded on unpaired episodes); `--no-race-episodes` keeps
pairing and judges every genome on every trial. The arms are in §4 of [REVIEW.md](REVIEW.md).

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
vertical level (see [the level format](#level-format)), and `scratch-trap-tiles.json` is
the two trap tiles on one floor — a cracked span over a pit with three pads at the bottom
of it. `train-cli render --level levels/scratch-trap-tiles.json --action run` shows both
in a terminal: the deck goes out from under the runner's back foot, and the pads throw it
out of the pit and into the flag. `scratch-trap-crossing.json` is a *route* rather than a
demo — the only way to its flag is a cracked span over a spiked pit, with a hole in the
middle of the deck to take on the fly — and it is the level whose learning measurement is
in [FINDINGS.md](FINDINGS.md#the-trap-crossing-does-a-population-learn-the-crumbling-bridge).

A level's route graph and the oracle's verdict are derived from its terrain, so both are kept
beside the level as `<level>.json.routes` and reused by the next process that opens it — the window
and every `train-cli` command ask the same two questions of the same level, and the answers are
milliseconds of real physics each (finding 59: a reopened level costs a 53 µs read instead of
18.7 ms of sweeps). The files are git-ignored and keyed by the level's content hash and the build's
version pair, so saving an edit moves the key and they rebuild themselves; deleting one costs that
same few milliseconds once.

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

Tiles: `.` empty, `#` solid, `S`/`B`/`W` sand, brick and timber (all solid exactly as
`#`, and a different material for the *view* only), `^` spike (lethal, not solid), `G`
goal (normalized into a goal entity at load time).

**The two tiles that do something** are written as a material's lowercase self and as the
spike inverted, so the alphabet carries the rule:

| Tile | Glyph | What it does |
|---|---|---|
| cracked ground / sand / brick / timber | `d` `s` `b` `w` | solid — brick for a body to stand on, and **gone `CRUMBLE_TICKS` (32 ticks) after one does**. A span of it is a bridge with a deadline: 3.2 tiles crossed at a run, 4.8 at a sprint, which is the whole of the number. `d` is *dirt*, because `#` is the one material with no letter to fold |
| spring pad | `v` | not solid. A body that **lands** on it is thrown upward at `spring_v0` and keeps the horizontal speed it arrived with, so a pad is a redirection and not a speed the level hands out |

A cracked tile is drawn as the material it is made of plus a crack, so the level's own art
says which material is about to give way; a pad is drawn whole, like a spike. Both are
*behaviour*, not material, which is why they are the two things `render` and the editor's
palette show as their own cells.

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
which ability the level depends on — disabled-sprint, disabled-wall-climb and
disabled-pad runs (`ReachOptions { spring: false }`) are how the pack keeps `l7-ascent`,
`l7-wall-school` and `l8-long-jump` honest about teaching what they are named for, and how
a level whose only crossing is a spring pad says so rather than being called unbeatable
(finding 60).

## Physics bible

All constants live in `sim::PHYSICS` and the jump capabilities are *derived* from them,
never tuned separately:

| Quantity | Value |
|---|---|
| `gravity` | 0.0125 tiles/tick² |
| `jump_v0` | 0.25 tiles/tick |
| `spring_v0` | 0.35 tiles/tick — a spring pad's throw |
| `run_speed` | 0.10 tiles/tick (6 tiles/s) |
| `sprint_speed` | 0.15 tiles/tick (9 tiles/s), ramped at 0.005/tick² from `run_speed` |
| `max_fall` | 0.75 tiles/tick |
| jump height | `v0²/(2g)` = 2.50 tiles continuous, **2.375 measured** |
| spring height | `spring_v0²/(2g)` = **4.90 tiles** — two jumps and a bit, and the pad's launch zone is as tall as `max_fall` so no fall can tunnel through it |
| air time | `2·v0/g` = 40 ticks |
| spring air time | `2·spring_v0/g` = **56 ticks** |
| jump distance | `run_speed · air_time` = 4.00 tiles |
| sprint jump distance | `sprint_speed · air_time` = 6.00 tiles |
| spring carry | `vx · 56`: **5.60 tiles** at run speed, **8.40** at sprint — a pad keeps the `vx` the body brought, so its carry has no single answer |
| brittle fuse | `CRUMBLE_TICKS` = 32 ticks (eight decision blocks) from a body standing on a cracked tile to that tile leaving the level |
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
* **A spring pad throws a body that *lands* on it, and only one that is falling.** The pad
  writes the body's vertical velocity and nothing else, so the run a body arrived with is
  the run it leaves with, and a body rising through a pad from below — or walking across one
  on the floor — passes through untouched. That is one rule rather than two, and it is what
  makes a pad a target to land on rather than an obstacle in a corridor. The launch spends
  the body's jump cut as it fires, so a pad delivers its full derived 4.90 tiles rather than
  the 45% a cut ascent would leave (REVIEW.md §1.10).
* **Brittle ground is ground until it is not, and `CRUMBLE_TICKS` starts when a body
  *stands* on it.** A body crossing a cracked span at running speed is past each tile long
  before its fuse burns out, so a bridge is crossed at a run and lost by hesitating; a walker
  patrol never lights one at all, because a patrol is a machine walking the same three tiles
  for the whole episode and would demolish the level it lives in. The tile leaves the level as
  plain air, through the same `set_tile` the editor uses, so the route cache and the state
  hash both see the terrain the body changed.

## Development

```bash
cargo test --workspace                  # 563 tests: sim, evo, render, train-cli, app
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
