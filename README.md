# Evolution Platformer

Build a 2D platformer level, then watch a population of neural networks evolve to
beat it. The entertainment product is the **visible learning arc** — stands still,
walks, jumps, beats the level — not the algorithm.

This repository is the implementation of [development_plan.md](development_plan.md).
Read that first; it is the design document, and this README records what the code
currently does and where it deviates.

## Status

| Milestone | State |
|---|---|
| **M0** scaffolding | done |
| **M1** sim + hand-written levels | done |
| **M2** GA + headless training + reachability oracle | done |
| **M3** watch mode + fun gate | **Tiers 1 and 2 done, fun gate open** — the owner's call |
| **M4** editor + content | **the shell, the evolution view and the replay work** — pixel renderer, generated art, drag-and-drop editor, in-editor playtest, and the *whole* watch mode hosted in the window: chart, lanes, route scrubber, champion saving, stall fallback and checkpoint resume (findings 24–28), the run's best agent replayed at real time with its generation ghosted (finding 31), and a title screen that plays the game while it introduces it (finding 41). Not yet: the curated starter pack and a browsable level shelf |
| M5 web/release | not started — except M5's stall chain, done early (findings 19–22) |

The deterministic core, the evolution loop, the reachability oracle and the Tier 1
watch mode are implemented and tested. `train-cli watch` grows a population on
screen: fitness chart, best-4 + random-4 agents per generation, speed control,
pause, death markers and the genius-agent splash. The level pack behind it is now
six levels wide, and its two hardest are *hard* (findings 15–18) — hard enough that
they sometimes stall, which the view reports with a stall line naming how long the
population has been stuck and which obstacle it keeps dying on (finding 19), and
offers a live mutation-rate knob to try instead (finding 20). A run can also be told
which easier level to fall back to: the first stall writes the stuck population out
as a checkpoint and trains it there for a while before returning, which turns one
run that never solved L5 in any number of generations into one that solves it
(finding 22) — and since finding 28 that step, its checkpoint and a resume from it are
all in the window too, so the terminal is a second front-end rather than the only one.
Every generation that
improves the best also has its champion route recorded, and the arrow keys scrub
back through those routes — position by position — to the moment a jump was first
learned (finding 21). What
remains of M3 is the part no test can answer — whether watching it is fun — and
that needs a human at a keyboard. See [The M3 problem](#the-m3-problem).

The same run now happens in the window. `cargo run --release -p app -- --evolve`
puts the trainer on its own thread, animates the generation it just finished over the
terrain (champion solid, the other seven lanes ghosted and colour-tagged), draws the
fitness chart as real geometry rather than a character plot, and lets the arrow keys
scrub the recorded champion routes. It is the terminal mode's own design — one
generation, then the animation, then the next — not a second evolution loop
(findings 26–27).

## Measured numbers

The plan extrapolated "~1–3 s per generation for pop 200 on 6 cores" from a 2009
benchmark and asked for the real numbers early (M2). Real numbers, on the target
i5-9400 (6 cores), `--release`, pop 200: **~0.014 s per generation**, ~14 000
population-episodes per second — roughly 100× faster than the plan's estimate. The
plan said to profile if the numbers were far off; the profile is that nothing needs
optimising. The consequence is a *product* problem, not a performance one: a solve
is over before the screen has finished drawing, so watch mode cannot be "run
training at 50× speed" and the levels have to carry the arc themselves. See
[The M3 problem](#the-m3-problem).

### Difficulty: the generation the population first wins

Every shipped level, twelve seeds each, pop 200, `--stop-on-win`, cap 420
generations (§9's "the population is itself the difficulty oracle"):

| Level | Tiles | Solved | Median | Fastest | Slowest | Won at generation 0 |
|---|---|---|---|---|---|---|
| L1-flat-run | 60 | 12/12 | 0 | 0 | 0 | 12 of 12 seeds |
| L2-first-gap | 60 | 12/12 | 0 | 0 | 0 | 12 of 12 seeds |
| L3-gauntlet | 60 | 12/12 | 11 | 3 | 30 | never |
| L4-rhythm | 67 | 12/12 | 16 | 11 | 22 | never |
| L5-tightrope | 70 | 10/12 | 68 | 41 | 279 | never |
| L6-marathon | 80 | 11/12 | 40 | 14 | 237 | never |

Reproduce a row with:

```bash
for seed in $(seq 1 12); do
  ./target/release/train-cli train --level levels/l5-tightrope.json \
      --generations 420 --pop 200 --seed $seed --stop-on-win --quiet --out /tmp/out \
    | grep -o "solved at generation [0-9]*"
  echo
done
```

L1 and L2 are smoke tests — a randomly initialised population already wins, so
their interesting minute is over before the first frame is drawn. L3–L6 are the
earned half of the pack, and the two hard levels are hard *most* of the time rather
than always: ten and eleven seeds of twelve, the rest stalling past 420
generations. That shape is deliberate; finding 15 explains why a stall is level
information rather than a bug.

### What the ladder costs to watch

Rendered frames per generation at pop 128, counted from `--headless` frames (a real
terminal draws them at 60 Hz, so 16 ms each, see finding 10):

| Level | `--speed 8` (default) | `--speed 50` | Median solve at 8× | Median solve at 1× |
|---|---|---|---|---|
| L4-rhythm | 163 frames (2.7 s) | 26 (0.43 s) | 43 s | 5.8 min |
| L5-tightrope | 194 frames (3.2 s) | 31 (0.52 s) | 3.6 min | 29 min |
| L6-marathon | 400 frames (6.7 s) | 64 (1.07 s) | 4.5 min | 36 min |

A generation lasts as long as its slowest showcase lane, so the longer and harder a
level is, the more frames it needs — which is why L6 costs more per generation than
L4 even though its median solve is lower. These are *generation-1* numbers, drawn
before anything has been learned: once a population starts dying at the obstacle
blocking it, its episodes get shorter and its generations get cheaper, so read the
table as the cost of the opening minutes of an arc, not of all of it. The point for the M3 gate: the same speed
control now spans "over in seconds" to "half an hour", so the two hard levels can be
watched slowly enough to see *where* the population is stuck. Before this pack, no
speed setting helped, because L1/L2 were solved by the initial population and L3 was
over in under a minute at every speed.

## Workspace

```
crates/sim/        the deterministic core: tile grid, AABB physics, sensors, oracle
crates/evo/        fixed-topology genome, GA, fitness, champion files
crates/render/     read-only views: ASCII frame/scene, keyboard play, pure text charts
crates/train-cli/  headless driver: train, watch, reach, replay, play, render, sensors
crates/app/        the game: pixel renderer, drag-and-drop level editor, playtest
levels/            hand-written *.json levels (L1-L6, easy → brutal)
assets/            generated pixel art: the tileset atlas, its manifest, a preview
tools/art/         the art itself: terrain, characters, props, one module each
tools/             asset generator (python3 tools/gen_tileset.py)
champions/         generated champion files (git-ignored)
```

The plan's §4 layout put the editor at `crates/editor`; it is `crates/app` here
because it grew past an editor into the game shell — it owns the window, the pixel
renderer, the editor *and* the playtest, and evolve/watch will move in beside them.
The two toolchains stay separate on purpose: `train-cli` remains the headless,
no-window, all-cores path, and nothing in `sim`/`evo`/`render` depends on a GUI.

Boundary rules from the plan (§4), enforced by review:

1. `crates/sim` has no `std::time`, no threads, no `HashMap` in hot paths, and no
   I/O beyond explicit JSON parsing. Its only dependencies are `serde`,
   `serde_json` and `rand_xoshiro`.
2. Everything the sim needs crosses the boundary as plain data: `&Level`,
   `AgentAction`, `seed: u64`.
3. `crates/render` only ever borrows `&Sim` and simulates nothing itself. One
documented exception: `render::chart` depends on nothing at all — charts are
numbers in, text out — so the learning curve can be drawn and unit-tested without
physics. `watch` is a composition of a scene view and a chart, not a fourth
renderer.

## Quick start

```bash
cargo test --workspace                 # 381 tests
cargo build --release

# The game: draw a level, drag tiles out of a palette, play it yourself.
python3 tools/gen_tileset.py           # only if you change the art
cargo run --release -p app             # opens on the title screen, then l1-flat-run.json
#   any key or click leaves the title screen for the editor. Scripted frames skip it by
#   default (--no-splash is a no-op there); --splash asks for it in front of one anyway.
#   The screen plays the run's saved champion for the level being opened whenever
#   champions/ holds one that beat it — which is what makes it worth watching a second
#   time — and a demo run when it does not (see finding 42).
cargo run --release -p app -- --level levels/l6-marathon.json
#   drag a palette item onto the level to place it; click to paint
#   right-drag erases   middle-drag (or alt-drag) pans   wheel zooms
#   ctrl+Z / ctrl+Y undo   G grid   F fit   ctrl+S save   P play/pause-mode
#   in play: arrows or W/A/S/D move and jump, R restarts, P or Esc returns

# The same run inside the window: train on the level you are editing and watch it.
cargo run --release -p app -- --evolve --level levels/l2-first-gap.json
#   space pause   n one generation   -/+ speed   m/M mutation rate
#   left/right scrub the recorded champions   h back to live
#   s save the best-ever champion   f follow the leader/whole level
#   P or Esc back to the editor
cargo run --release -p app -- --evolve --pop 60 --seed 5 --generations 8 \
    --screenshot out/evolve.png

# Watch the run's answer rather than its search: the best agent it found, playing the
# level at the speed the game runs at, with the population it last scored ghosted beside
# it (finding 31). V enters it from the editor or the evolution view.
cargo run --release -p app -- --replay --generations 20 --level levels/l3-gauntlet.json
#   the replay opens at 1x real time, fitted to the whole level: the field is the point,
#   and a camera pinned to one agent hides most of it
#   space pause   R run it again   G ghosts   +/- speed   F follow the champion (F again
#   gives the whole level back)   V or Esc back to the run
#   the field is drawn in four rank bands — cyan for the best of the population, through
#   violet and amber to salmon for the stragglers — and the panel legend names the ranks
cargo run --release -p app -- --replay --generations 20 --no-ghosts \
    --screenshot out/replay.png        # the champion alone, for a clean frame
# A replay is the episode the run already scored, not a fresh one: it ends on the fitness
# its champion file records, and `--replay` with `--generations` saves that file too.

# The window takes the terminal's own run flags, with the terminal's meanings, so a run
# can be started in one front-end and resumed in the other: the same stall fallback
# (finding 22/28), and the same checkpoint file either way.
cargo run --release -p app -- --evolve --level levels/l5-tightrope.json \
    --curriculum levels/l3-gauntlet.json
cargo run --release -p app -- --evolve --level levels/l5-tightrope.json \
    --resume champions/l5-tightrope.population.json
# A run with --generations saves its best genome when it stops, exactly as `watch` does.

# Render one frame to a PNG and exit (how the renderer is verified without a human).
cargo run --release -p app -- --screenshot out/frame.png
# The title screen is a still the same way, captured finished: its reveal takes a second and
# a half and a screenshot cannot show an animation, so it is advanced to its end first. Point
# it at a level whose champion lost (lots of them) to photograph the demo run instead.
cargo run --release -p app -- --splash --screenshot out/splash-champion.png
cargo run --release -p app -- --level levels/l5-tightrope.json \
    --splash --screenshot out/splash.png

# Can this level be beaten at all? Milliseconds, no training.
./target/release/train-cli reach --all --levels-dir levels --verbose

# Watch it learn: chart, best-4 + random-4 agents, deaths, genius splashes.
./target/release/train-cli watch --level levels/l5-tightrope.json
#   space pause   -/+ speed   m/M mutation rate   n next generation
#   left/right scrub the recorded champions   h back to live
#   f follow/whole level   s save champion   q quit

# A run that stalls checkpoints itself and steps back to an easier level, then
# returns to the hard one with the population it grew there (finding 22).
./target/release/train-cli watch --level levels/l5-tightrope.json \
    --curriculum levels/l3-gauntlet.json
# Restart from a checkpoint written by such a step, on whatever level you like.
./target/release/train-cli watch --resume champions/l5-tightrope.population.json \
    --level levels/l5-tightrope.json

# Evolve headlessly, print the fitness chart, save a champion.
./target/release/train-cli train --level levels/l3-gauntlet.json \
    --generations 200 --pop 200 --stop-on-win --csv out/l3.csv

# Re-simulate a saved champion and verify it reproduces its trajectory exactly.
./target/release/train-cli replay --champion champions/l3-gauntlet.json

# Human playtest, in the terminal.
./target/release/train-cli play --level levels/l1-flat-run.json

# ASCII frames of a level with a fixed action, for eyeballing physics.
./target/release/train-cli render --level levels/l3-gauntlet.json --steps 400 --stride 40
```

`train-cli --help` lists every option.

## Determinism

The determinism contract (§5) is the spine of the project: the same genome and
the same level must produce the same fitness, or selection is noise.

* Fixed timestep `DT = 1/60`; headless training steps in a plain loop.
* One seeded `Xoshiro256PlusPlus` per episode; no `thread_rng`, no wall clock in
  the sim. `episode_seed(master, generation, genome_id, trial)` is frozen and
  pinned by golden values.
* Parallel evaluation uses rayon Pattern A: the RNG is built *inside* the
  `par_iter().map()` closure, and `collect()` preserves order, so `results[i]`
  always describes `genomes[i]` whatever order threads finish in. A test compares
  a 1-thread pool against a 6-thread pool and requires identical fitness vectors
  and trajectory hashes.
* `sim::Sim::state_hash()` hashes the whole state, canonicalizing NaNs, and is
  pinned by two golden tests: a 10 000-frame run and a fixed observation vector.
  Any change that moves either value must be a deliberate version bump.

### The version trio

`sensors::VERSION`, `evo::fitness::VERSION` and `sim::SIM_VERSION` are stamped
into every champion file along with `action_repeat` and the level hash. Loading
refuses a champion whose versions or level hash do not match, so an old champion
can never silently misbehave (§8).

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

Tiles: `.` empty, `#` solid, `S` sand (solid exactly as `#`, and a different
material for the *view* only), `^` spike (lethal, not solid), `G` goal (normalized
into a goal entity at load time).

**Coordinates are tile coordinates, and an entity at `(x, y)` stands on the
*bottom* edge of row `y`.** An entity at `y = 12` resting on ground tiles in row
`13` is written `y = 12`. Out-of-bounds tiles are solid to the left and right
(invisible level walls) and empty above and below (you may jump off the top;
falling out of the bottom is a pit death).

## Physics bible

All constants live in `sim::PHYSICS` and the jump capabilities are *derived* from
them, never tuned separately:

| Quantity | Value |
|---|---|
| `gravity` | 0.0125 tiles/tick² |
| `jump_v0` | 0.25 tiles/tick |
| `run_speed` | 0.10 tiles/tick (6 tiles/s) |
| `max_fall` | 0.75 tiles/tick |
| jump height | `v0²/(2g)` = 2.50 tiles continuous, **2.375 measured** |
| air time | `2·v0/g` = 40 ticks |
| jump distance | `run_speed · air_time` = 4.00 tiles |
| shortest possible jump | 1.70 tiles (`min_jump_hold` 8 ticks, then `jump_cut`) |

The discrete Euler integration in the sim peaks slightly lower than the
continuous formula; `PHYSICS::discrete_max_jump_height()` returns the real
number and is what the reachability diagnostics quote.

## Agent interface

**Sensors (v1, 90 inputs).** A 9×9 tile patch around the body (solid `1.0`,
spike `-0.5`, enemy `-1.0`, empty `0.0`), then nearest-enemy `dx`/`dy`, a
`gap_ahead` probe, `grounded`, normalised `vx`/`vy`, `can_jump`, goal-direction
sign, and a bias input. The plan's sensor table omits spikes, which would leave
the most common hazard invisible; `-0.5` fills that gap and is documented in
`sensors.rs`.

**Actions (6 macro-actions).** `run`, `run_jump`, `retreat`, `brake`,
`jump_in_place`, `walk`, chosen by argmax over 6 network outputs and held for
`action_repeat` (default 4) physics ticks.

**Network.** 90 → 16 (tanh) → 6 (tanh), 1558 genes. Fixed topology; NEAT is
deferred to M5 and gated on an observed plateau (§2.1).

**Fitness (v1).** `+1.0` per new rightmost pixel, `+0.05` per frame alive,
`-120` on death, `+10 000` on touching the flag. Episodes end after 600 frames
without new ground (anti-camping) or `width × 40` frames absolute. No proxy
rewards: nothing scores jump height, airtime or stomps.

## Reachability oracle

`sim::reach` answers "is this level beatable at all?" in milliseconds, before any
training budget is spent. It extracts platforms, sweeps canonical take-offs
(every tile column × 20 sub-tile offsets at 0.05 granularity × 3 directions ×
jump-hold variants), simulates each with the **real** physics stepper and collision
query, and BFSes from the spawn platform. On failure it names the first jump that
cannot be made and why ("gap too wide", "wall too tall").

Its jump windows are a **lower bound** on what an agent can do (finding 18), so read
it as "beatable at all", never as "hard" or "easy": it is a safety check on a level
before a training budget is spent, not a difficulty meter. All six shipped levels
pass it, including L3's spike patch, which the coarse five-offset sweep used to
report as impassable (finding 6).

## Testing

381 tests, all green. `cargo test --workspace`.

The load-bearing ones: `determinism_hash` (10 000 frames twice → identical hash,
plus a pinned golden), `parallel_determinism` (1 thread vs 6 → identical fitness
vectors), `golden_observation_hash_at_a_fixed_state` (the 90-float vector is
pinned), champion round-trip (save → load → re-simulate → identical trajectory
hash), the reachability cases (a 3-tile gap passes, a 5-tile gap reports "gap too
wide", a 5-tile wall reports "wall too tall"), physics corner cases (landing,
head-bump, wall-slide, stomp vs side-hit, spike death, pit death, goal trigger),
and the GA smoke test.

Content gets its own guard rails, because level JSON is content and nothing else
checks it: every shipped level must load with the width, height and walker count the
pack table declares, be reachable in principle *with no caveats*, terminate inside
its frame cap, and keep its entities inside the grid and its walkers standing on
something (finding 17's off-grid walker is the mistake that test exists for). The
pack's difficulty is pinned by `crates/evo/tests/difficulty.rs`: a random
population must win L1/L2 at generation 0, must win none of L3–L6 on any of four
seeds, and L4 must still fall to evolution inside 60 generations at pop 64.

The checkpoint gets five: a population round-trips with its ids and weights intact,
stale versions and wrong network shapes are refused, a duplicated or missing genome
is refused, a *different* level is reported rather than refused (that is the whole
point of resuming one), and a nested path that does not exist yet is created on save.
`Trainer::adopt` — how the curriculum hands its population back — is pinned by two
more: the adoption changes the next generation while leaving the chart, the champion
and the id counter untouched, and a population that would break the run (wrong size,
duplicate ids, malformed genome) is refused. Watch mode's own end-to-end test runs a
stalled L5 through `--curriculum`, asserts the checkpoint exists and names the stall
that produced it, then restarts from that very file and asserts the champion it saves
carries an id out of the checkpoint. The window runs the same test from its own side of
the boundary (`crates/app/src/lab.rs`): a stalled run writes its checkpoint, hands the
population back, says so once and keeps its own chart, a run without an easier level
stalls without writing anything, a resumed run continues the checkpoint's generation and
genomes, and `--resume` on a file that cannot be read refuses rather than training
something else. Two of those tests only exist because the rule moved out of the window:
the follow camera's leader (`lab::leader_centre`) and the curriculum's sentence
(`evo::CurriculumStep::note`) are now functions, not lines inside a frame.

The recorded champion paths get six of their own in `render::trace` — the log keeps
an improvement and drops the flat generations, refuses a non-finite fitness (a NaN
would refuse every later trace, an infinity would freeze the record), steps between
recorded generations rather than counting up by one, stops at the oldest instead of
wrapping, returns to live when the record it was showing ages out, and draws one
marker per cell with the end on top. Watch mode adds eleven that hold its honesty
in place:
`every_showcase_lane_replays_the_episode_that_was_evaluated` (a lane animates the
episode that was scored, seed for seed, so the picture and the numbers agree),
`watching_does_not_change_what_is_evolved` (the showcase's RNG is separate, so
training is identical whether or not anyone is looking), `pausing_freezes_the
physics_and_stops_burning_the_frame_budget` (a paused generation may not advance),
and two on the stall line: `the_stall_line_waits_for_the_cutoff_and_clears_on_an_
improvement` (25 flat generations, no fewer, and a new best removes it again) and
`the_stall_line_names_the_stall_and_the_death_cluster` (the row carries the count,
the place deaths pile up, and replaces the trend caption instead of contradicting
it). Two more hold the live mutation knob honest: `the_live_mutation_rate_changes_
the_offspring` presses `m` and asserts the *next generation's genomes* actually
differ from the unpressed run — the key changes the evolution rather than the label
— and `the_mutation_ladder_stays_inside_the_plans_range` pins every rung to the
documented 0.03–0.10 and checks the ends clamp instead of wrapping. Four more cover
the scrubber: `the_record_keeps_the_route_the_run_actually_scored` re-simulates the
recorded episode from the genome and seed it was scored with and asserts the route
ends on that episode's last frame and outcome (the same fidelity rule the lanes
live by), `the_scrub_keys_walk_the_record_and_return_to_live` drives `←`/`→`/`h`,
`the_record_row_reports_the_record_and_labels_the_scrubbed_view` checks that a
scrubbed frame describes the record and never the live lane count, and
`a_scrubbed_view_draws_the_recorded_route_without_the_live_lanes` asserts on the
frame that actually reaches the screen.
The watch layout itself is testable without a TTY because `--headless` renders
plain text frames; `cargo test` exercises the whole frame composition that way.

The playtest mode gets its own file. `render::play`'s seven unit tests pin the
keyboard layer's mechanics without a terminal — that a press latches for the window
and a release ends it at once, that the six actions are chosen in the plan's priority
order (jump beats direction, so a held direction while jumping is a real forward
jump), that quit and restart fire once rather than every frame, that a five-presses-a-
second repeat runs the player across tiles instead of pixels, and that with no press
at all the player does not drift. `crates/render/tests/playability.rs` then measures
the conjunction that matters — the keyboard layer against the shipped levels: one
latched press must clear L2's first gap over a take-off window of at least 192 ms
(the shipped constants measure 224 ms; a 150 ms latch fails the test), the gap must
stay fatal without a press (or the test measures nothing), holding right must beat
L1, and a player who jumps when they see an obstacle must clear the first gap at some
reaction latency inside 240 ms. That last one is the M1 exit criterion in executable
form, as far as a model of a player can carry it.

The desktop shell gets forty-six, and they exist because two of its failure modes are
invisible from the outside. `crates/sim`'s editing API has twelve of its own, the one
that matters being `every_shipped_level_round_trips_through_the_writer_with_the_same
_hash` — the hash covers the name, the grid, the spawn and every entity box, so a
tile column that rounds the wrong way when an entity's position is converted back to
a tile fails there instead of shipping a level that plays differently. The app's
atlas tests check that every sprite maps *inside the texture* at exactly one cell
(the bug in finding 25 rendered a level as nothing at all), that the shipped art
carries every name the renderer asks for, and that a manifest disagreeing with the
image is refused rather than drawn as confetti. Its view test draws a shipped level
into a headless egui context and counts the sprite quads that come out — a real frame,
no window, no GPU. Its editor tests hold the rules the user depends on: a drag paints
a gapless line as one undo step, the eraser takes an entity before the terrain under
it, the last goal cannot be erased away, a refused save leaves no file behind, and the
beatable/not-beatable verdict is the oracle's answer following a spike curtain drawn
across the approach. Its playtest tests pin the accumulator (a 30 ms frame must not
lose physics, a two-second hitch must not teleport the player), that holding right
beats L1 through the real input path, and that a restart is the same seeded episode.

The evolution view adds twenty-five more, and they are about the two things that
would make it a lie. The first is *fidelity*: `a_lane_replays_the_episode_that_was_scored`
animates a real generation's lanes to their end and checks each one's own score against
the fitness printed beside it, and `a_recorded_route_ends_where_the_champion_ended`
re-runs a recorded route's genome and seed and requires the same ending — the numbers on
screen have to be the numbers being drawn. The second is *rate*:
`a_run_waits_for_the_viewer_between_generations` parks a real worker thread, sleeps 200
ms, and requires that nothing else was trained — a run that trains on a timer is the one
behaviour the whole design forbids — then asks for the next generation and requires it.
The rest cover the packing (labels counted within their group, deaths tagged with their
generation and forgotten after the window), the ladders stopping at both ends, the
chart's geometry (a flat run, a single generation, non-finite values skipped, the highest
point highest, wins marked on the top rule only), a headless draw of the panel and the
lanes so a frame that renders nothing fails a test rather than shipping, and the key map
itself — every documented key to the command its button produces — which is asserted as
data precisely because pressing a real key in a real window is the one thing no test here
can do.

## Findings and deviations from the plan

Things the implementation learned that the plan did not anticipate. Each is a
deliberate, documented decision — not drift.

1. **`episode_seed` had no observable effect.** v1 gameplay consumed no
   randomness, so every trial of a genome was the byte-identical episode and
   `--trials` would have silently averaged N copies of one number. Fixed by
   drawing each walker's patrol phase (initial direction + sub-tile offset) from
   the episode RNG at `Sim::new`. That is v1's only stochastic element, so only
   walker levels vary with the seed — asserted in both directions.
2. **The all-zeros genome is a free "always run" policy**, because the argmax
   tie-break picks the lowest output index. This makes evolution start from "hold
   right" for free, and it means L1 is solved by *any* population at generation 0.
   The flat level is therefore a smoke test only; the GA tests use a pit fixture
   that a constant policy cannot pass.
3. **L1 and L2 are solved at generation 0.** They are useful as sign-bug guards
   and useless as evidence of learning. L3 (two pits, spikes, a walker) is the
   first level that actually requires evolution.
4. **Jumps fire while the button is *held*, not on a press edge.** With the
   action table re-queried every 4 ticks, an edge trigger forces the network to
   discover a release-then-press rhythm whose only effect is to make jumping hard
   to evolve. Holding through a landing simply re-jumps.
5. **A jump is force-held for 8 ticks** (`min_jump_hold`), so a 4-tick action
   repeat cannot shrink every jump to a 1-tile hop. Shortest jump is 1.70 tiles,
   full is 2.375.
6. **An agent can take off from anywhere inside a tile**, not just on a lattice.
   Sampling a single sub-tile offset made the oracle call L3's spike patch
   impassable; five offsets fixed most of it and twenty (0.05 granularity) made L3
   reachable with no caveats at all, which is what the shipped-level test asserts
   now. Getting the pre-take-off runway test backwards (checking beyond the ledge
   instead of behind it) made *every* pit uncrossable — the platform's outermost
   column is the one you clear a gap from, and it has no platform outside it.
   The finer sweep also caught a test that was passing for the wrong reason: its
   "unbeatable" fixture had a 4-tile pit — exactly the width the physics bible
   allows — with a goal floating one column past the far ledge, a combination the
   coarse sweep could not cross. It is a 6-tile pit now, and unbeatable for the
   reason its name claims.
7. **`level_hash` is `fnv1a64:<hex>`, not `sha256:<hex>`** as the plan sketches,
   to keep the workspace free of a hashing dependency for an integrity check.
8. **No `clap`.** `train-cli` hand-rolls its argument parsing to keep the
   dependency tree tiny; unknown options are rejected rather than ignored.
9. **No `rand` crate.** `rand_xoshiro` alone, with a hand-rolled Box–Muller, so
   the exact mutation stream is pinned by this repository's own code rather than
   by a dependency's choice of algorithm.
10. **Watch mode is deliberately *slower* than training, not faster.** The plan's
   premise was "watch training at 50× speed", but the benchmark inverted it: a
   solve takes 0.3 s, so speeding it up shows nothing. Instead each generation is
   evaluated once (14 ms) and then the eight showcase episodes are re-run tick by
   tick, and that animation *is* the pacing. `--speed` is physics ticks per
   rendered frame, exactly as Tier 1.4 specifies; a generation lasts as long as its
   slowest lane, capped at `level_frames / speed + 30` rendered frames so a survivor
   cannot hold the view hostage.

   Measured at pop 128, frames drawn per generation (a 60 Hz render loop, so
   16 ms each): L1 279 frames (4.5 s) at 8× and 45 (0.7 s) at 50×; L3 133 frames
   (2.1 s) at 8× and 22 (0.3 s) at 50×. The spread between L1 and L3 is the levels
   answering a different question — L1's agents survive to the flag while L3's die
   at the first pit, and a death ends a lane sooner than a win. In an interactive
   session at pop 32: 2 generations in 6 s at 8×, 10 in the same 6 s after three
   `+` presses.
11. **A terminal that reports 0 columns drew a blank screen.** `terminal::size()`
   returns `(0, 0)` under `script`, a CI shell or a detached multiplexer pane, and
   a zero-width frame is an *empty* frame — the alternate screen stayed blank and
   watch mode looked hung. Found only by running it under a real pty
   (`script -qec`), which is now part of how this is verified. Fixed by falling
   back to 80×30 when the size is unusable.
12. **Pause used to burn the frame budget.** The generation budget counted drawn
   frames, so pausing froze the agents while the generation still ticked toward its
   end; unpausing dumped the viewer into the next generation having seen nothing.
   The budget now counts *stepped* frames, so a pause holds a generation still
   indefinitely.
13. **The lane labels were numbered from the wrong thing.** `Trainer::showcase`
   numbers random lanes `best_k + n`, so deriving a glyph from `lane.rank` made the
   four outsiders `e`, `f`, `g`, `h` — and `e` is the walker glyph, which is how
   this was caught: a random agent was drawn as an enemy. Labels now come from each
   lane's ordinal *within its group*, and the random alphabet skips `e` and `x`.
   The lesson is bigger than the glyph: the champion lane is `1` and the eye tracks
   it, so lane identity is part of the UI contract and is pinned by a test.
14. **On a level with a walker, the generation max can fall after a win, and that
   is not degeneration.** `episode_seed` mixes in the generation (§5.5, deliberate:
   an elite must not keep a luckily easy episode forever), so on the one stochastic
   level an elite re-rolls the walker's patrol phase every generation and can lose
   the run that won. `best_ever` keeps the champion — a watched L3 run found a
   winner at generation 11 and still showed 567 as the max two generations later —
   but the chart's "max falling = degenerating" reading needs that caveat, and
   `--trials` (now meaningful) is the mitigation: it averages the noise instead of
   displaying one roll of it.
15. **A hazard the population cannot clear is a *trap*, not just a wall.** Fitness
   pays `+1` per new rightmost pixel and `+0.05` per frame survived, and charges
   `-120` on death — 7.5 tiles of progress. Once a jump is clearable only from a
   narrow window, parking five tiles short of it out-scores dying on it, so
   selection does not merely fail to find the jump: it actively removes whatever
   tried. Measured: a maximal pit followed by a spike patch at the end of a level
   was never solved in 420 generations at pop 200 (the best genome parked at
   `x = 52`, 1117 frames survived); the same level with the last spike patch gone
   fell at generation 36. The oracle calls both beatable and is right — a canonical
   jump crosses the patch — but it cannot say whether *selection* can find that
   jump. This is the floor under "harder levels": difficulty has to be discoverable
   in steps, which is why the hard levels in the pack are hard *most* of the time
   rather than always.
16. **What makes a level hard here, measured.** Single obstacles are nearly free:
   one 4-tile pit falls at generation 0–2, a chain of eight 3-tile pits at
   generation 1–3, six wall hops at generation 0 (the initial random population
   bunny-hops over them), and four spike patches spaced over generous ground at
   generation 4–17. The one atom that resists is a **maximal pit, a two-tile pad,
   another maximal pit**: 10–106 generations for a single instance, 10–100× every
   other obstacle. Chaining *different* easy obstacles stays easy (six mixed
   obstacles: median 7–17 generations); chaining four hard ones was never solved at
   all. The pack is therefore built to a budget: one or two boss atoms, at least 16
   tiles of runway before the first one, 2-tile pads (3-tile pads roughly halve the
   difficulty), spike patches only where the approach is long, and a finish run long
   enough that a winner is not killed at the flag.
17. **The enemy sensor sees through the whole level.** Slots 81–82 hold the nearest
   live walker's `dx, dy`, normalised by `ENEMY_VIEW_DISTANCE = 4.0` and clamped, so
   a walker 80 tiles away reads exactly like one 4 tiles away, and any level with a
   walker in it hands every agent a constant "enemy that way" input from tick zero.
   An *off-grid* walker does too: it freezes in mid-air (walkers only patrol where
   they are supported) yet is still the nearest enemy, and adding one to a finished
   level changed 56 of 200 generation-0 fitnesses. A "no enemy in view" state would
   be `sensors::VERSION` 2, so this is documented and pinned by a test rather than
   changed, and the shipped-level tests now refuse entities outside the grid or
   standing on nothing.
18. **The oracle's jump windows are a lower bound, not a difficulty meter.** The
   canonical model jumps from a standing start at full run speed; the sim also
   allows a take-off inside the 4-tick coyote window *after* walking off a ledge,
   which is strictly more reach than the oracle samples. So a lone 4-tile pit is
   crossable in the oracle from only ~3 sub-tile positions at its ledge column,
   while evolution crosses it at generation 0 — the real window is far wider. Hence
   the split of duties: the oracle answers "beatable at all", and the population
   answers "how hard" (§9's own suggestion).
19. **A stall needs saying out loud, and the label has to be present-tense.** The
   plan schedules stall UX for M5, but the shipped hard levels stall in practice, so
   the watch view reports it now: `stalled: no new best in a long time` on the status
   line, and a row under it with the count and the standing best — plus, when the
   rolling death window has anything in it, the tile column deaths cluster at.
   Measured windows at pop 200: L5 seed 1 stands still from
   generation 29 to 75 and *then* solves at 76; L5 seed 3 stalls three separate
   times (31–90, 116–183, 209–238) and still wins; L5 seed 5 never moves after
   generation 38. So the row is a statement about the present, never a verdict — a
   stalled run is worth continuing to watch, and its breakthrough is exactly the
   drama the M3 gate is asking about. The rule is "no improvement of 5% over the
   standing best for 25 generations", both numbers taken from those measurements:
   with a strict flatness test instead, the elite re-rolling its episode on a walker
   level (finding 14) would reset the clock forever, and a shorter cutoff would fire
   on runs that are simply slow.
20. **The mutation rate is a real knob, and it is not what unsticks a stall.** Tier
   2.9 asks for live mutation-rate tuning; measuring it gave two different answers to
   two different questions. *As a launch setting* it decides outcomes: on L5 seed 5 —
   the seed that never solves at the default rate, flat from generation 38 —
   `--mut-rate 0.03`, `0.08` and `0.10` all reach the flag (generations 31, 11 and
   65), while `0.05` fails all 420. *As a mid-run rescue* it does not work: switching
   a stalled run to 0.08, 0.10 or 0.12 at generation 45 left it unsolved at 420 —
   0.08 reached a best of 760 and the others 587 and 587, against a control of 587
   and a winning fitness of ~11 100, so the switched populations kept exploring and
   still never found the jump. That is finding 15 seen from the
   operator's seat — the park trap is the *criterion* rewarding "stand still and
   survive" (26.3 of 100 units are progress), not the search's step size, so no
   amount of local jitter escapes a peak the fitness function itself points at. The
   key is shipped because the rate genuinely matters and the plan asks for it, and
   the honest reading is that it belongs in the launch settings more than in the
   rescue kit: what would end a stall is a different landscape (resume from the stuck
   point on an easier level, M5), not a different schedule. The ladder is clamped to
   the plan's own 0.03–0.10 and snaps `--mut-rate` onto it, so the number in the
   header is always the number being used.
21. **A timeline is worth keeping only where it changed.** Tier 2.7 asks watch mode
   to record the champion's trajectory per generation and let the viewer scrub back
   to the moment the jump was learned. Recording every generation would keep 420
   near-identical routes on the seed that stalls (finding 19) — the rescue is
   invisible in exactly the case the scrubber exists for — so the record keeps the
   generations that moved the best by at least 1% and drops the rest. Measured on
   the two hardest seeds at pop 200 for 420 generations: **8 entries spanning `gen
   0..38`** on seed 5 (the seed whose best never moves again) and **5 entries
   spanning `gen 0..76`** on seed 1, whose solve at generation 76 is therefore the
   record's *last entry*. Two independent measurements agreeing on the same
   boundary is the strongest check this feature has: the record's span is the
   plateau, so a viewer who reads `record 8 improvements kept (gen 0..38)` at
   generation 400 is being told the same thing the stall line says, from data that
   was collected by different code. The cap is 64 entries (~1 MB), which those runs
   never reach; when it does fill, the oldest entry is dropped and the row shows the
   span, so a record that no longer reaches back to generation 0 says so rather than
   looking complete. Two smaller decisions are recorded here because they are the
   kind that get silently re-litigated: each recorded route is re-simulated as its
   own episode rather than sampled off the animation (a paused, skipped or
   frame-capped animation would truncate the path exactly when a viewer has walked
   away), and the trail's `:` is named on the row that owns it because the death
   strip already uses `:` for "one death in this column".
22. **A stalled population is the best starting material a run has: move it, do not
   reset it.** The last item in the plan's stall chain (§12 M5) is curriculum resume
   — "restart training from a checkpoint at the stuck point" — and since findings 19
   and 20 had already shown that the stall is real and that neither patience nor a
   mutation-rate change ends it, this is the lever that was left. Measured on
   L5-tightrope, pop 200, 420 generations, headless `watch`, one 40-generation detour
   taken the first time the stall fires:

   | seed | no detour | `--curriculum l3-gauntlet` | `--curriculum l4-rhythm` |
   |---|---|---|---|
   | 5 | **never solved** (best 587, flat from generation 38) | **solved at generation 163** | solved at generation 418 |
   | 3 | solved at generation 397 | solved at 184 | solved at 313 |

   The effect is **not extra compute**: seed 5 given 460 generations — the arm's 420
   plus its 40 detour generations — still never solves, and given 840, double the
   compute, its best is 589.82 against 586.98 at 420. The plateau is a fixed point;
   the detour is what breaks it. Nor is it that the excursion solves the level for
   the run: the detour reaches the *easier* level's flag at its own generation 71
   (L3) and 103 (L4), which says the stalled population can already perform the jump
   where the approach is shorter — L4 contains exactly L5's obstacle (4-tile gap,
   2-tile island, 4-tile gap) and the two differ only in the run-up, 27 tiles there
   against 30 here. That is a hypothesis about *why*; what is measured is that the
   population improves on the return and that the excursion learns on the easier
   level rather than merely being shaken.

   The design follows from what those numbers say, and each choice above is a
   decision not to do something:

   * **The checkpoint holds the population, not the champion.** A champion file
     records one genome and would have to fabricate a population around it; a
     checkpoint records all N genomes with their ids, because the ids are part of
     every `episode_seed` and a renumbered population is a different run (§5).
   * **The excursion is a separate trainer whose population is adopted back**, so
     the run keeps its chart, its best-ever champion and its RNG stream. That is
     what keeps the saved champion honest — every number in it was earned on *this*
     level, where a single shared trainer would have quietly contaminated them with
     the easier level's evaluations.
   * **The excursion is not animated** (0.6 s at pop 200, announced by a note). Forty
     seconds of a *different* level on screen would bury the generation the viewer is
     waiting for.
   * **One step per run**, because a second step from a population that has already
     had the easier level's lesson is unmeasured, and shipping an unmeasured policy is
     how a tool starts lying.
   * **A checkpoint's level hash is a note, not a gate**, unlike a champion's: a
     different level is what a curriculum *is*, so `--resume` prints which level the
     checkpoint came from and says plainly that the fitnesses either side are not
     comparable.

23. **The playtest mode could not be played, and no test could see it.** M1's exit
   criterion is "a human can beat L1–L3", and `play` is the tool that decides it —
   but the tool had only ever been driven by a keyboard, which is not a thing CI has.
   Driving it through a pty at a real terminal's auto-repeat rate (about five presses
   a second) found why a first playtest reads as "the controls are broken": the loop
   stepped **one frame per key press**, so holding a key moved the player at whatever
   rate the terminal happened to repeat. On L1, four seconds of holding right ended
   at `best x 2.40` — the spawn is at 2.0, so four tenths of a tile, a progress of
   3 px, where a held key covers 24 tiles in the same time. A press now latches its
   intent for 200 ms (a terminal that reports releases ends the latch early), the
   same four seconds reach `best x 26.9`, and twelve seconds of it reach the flag.

   What the fix had to be checked against is the *level*, and the instrument is the
   take-off window: the range of moments at which one press clears an obstacle.
   Swept on L2's first gap (columns 18–20, fatal without a press at x 20.05):

   | press | take-off window |
   |---|---|
   | one frame (16 ms) | 9 ticks, 144 ms |
   | one latched press (13 ticks, 208 ms) | **14 ticks, 224 ms** |
   | held down (640 ms) | 19 ticks, 304 ms |

   Two things fall out of that. The physics force-holds every jump for
   `min_jump_hold` ticks, so a one-frame press is not punished — the latch's
   contribution is the extra third of a second of *margin* above that floor. And the
   window belongs to the physics, not the keyboard: it does not narrow when the
   keyboard layer changes, which is why the test that pins it asserts the width a
   latched press buys (≥ 192 ms) rather than the latch's own length.

   The bound on M1's exit, as measured: a player who holds right and jumps whenever
   something appears in front of them wins L1 at every reaction latency up to 384 ms,
   and clears L2's first gap at some latency in 0–240 ms. Blind bunny-hopping wins L1
   and L2 outright and dies on L3, because landing on a spike is fatal. **L3 is not
   measured**: it chains a 3-tile gap, a 2-tile spike patch and a 3-tile gap with a
   3-tile landing zone between them, and placing those jumps is a judgement the model
   cannot make — it needs a person, which is the person the fun gate needs too.

24. **The game exists now, and it is the *renderer and editor* that had to be built
   first, not the evolution UI.** The owner's call was a native desktop app, a full
drag-and-drop editor and generated pixel art, so `crates/app` is an eframe window
with three deliberate properties:

   * **The art is data, not code.** `tools/gen_tileset.py` writes a 16x16-cell atlas
     and a manifest naming each sprite; the app reads the manifest, so replacing the
     art is replacing two files. A manifest that disagrees with the image's size is
     refused at startup, and a missing sprite is a startup error listing what the
     file does contain — a tileset that is *almost* right must fail loudly rather
     than draw mystery tiles.
   * **Terrain is picked by what surrounds it.** A solid tile is drawn from the 4-bit
     "open to the air on this side" mask of its neighbourhood, so a floor, a wall, a
     corner and an overhang are *different sprites* and a level needs no per-tile art
     decisions: sixteen soil masks, eight rock masks, and four decorated variants the
     view scatters by tile coordinate. Buried deep enough, soil becomes rock, and rock
     never grows a grass crown — that is what a pit's wall is made of.
   * **The keyboard has exactly one definition.** `render::play::action_for` is now
     public and both playtests call it: the terminal one latches a press, the GUI
     reads a held key, and neither can drift into being a different game.

   Two editor rules came out of the failure modes rather than from taste. The oracle
   is re-run when a *level hash* changes instead of behind a dirty flag, because a
   flag has to be remembered by every writer — including the ones that reach the level
   directly — and a hash cannot forget; the app caught a stale validation badge while
   the tests were being written. And an edit that cannot be saved is refused up front:
   the last goal cannot be erased and a shrinking grid drags its goal back in, so the
   document is never in a state where Save cannot work.

   The renderer is verified without a human: `--screenshot` renders one frame, writes
   a PNG and exits, and the pipeline asserts on the pixels. L1 renders roughly 18 700
   terrain pixels across its 60 tiles — grass over dirt, with the flag at the far end
   — and L3 renders 2 200 more plus the steel of its spikes, so the diff between the
   two frames is exactly the terrain and the spikes and nothing else. As of finding 29
   the *art itself* is checked the same way: the tests read the shipped PNG and assert
   that solid tiles are never transparent, that the grass crown is one flat colour
   across every mask that has sky above it, that variants only decorate the tile they
   vary, that characters stand on the bottom row of their cells, and that every frame
   of every animation actually differs from the others.

25. **A wrong unit rendered a whole level as nothing, and every test still passed.**
   `Atlas::uv` computed a cell's UV rect as `cell / cols` — 16 pixels divided by 8
   *columns*, i.e. 2.0 — where it needed the fraction of the *image*, `1 / cols`.
   Every sprite therefore sampled far outside the texture, so the level drew no
   terrain at all, while the code that resolved sprite names, chose animation frames,
   converted tiles to screen rectangles and counted what it had drawn all looked
   perfectly healthy: the frame was the right size, the panels were right, the grid
   lines and the spawn marker (drawn from rects, not images) were right. The pixels
   were the only witness, and every colour in the canvas was sky.

   The fix is one division; the lesson is the test. The atlas tests now assert that
   every sprite in the shipped tileset maps **inside** the texture at exactly one
   cell, and the view test counts the sprite quads a headless frame emits and where
   they land. Counting draw *calls* would not have caught this — the calls were all
   there; only the rectangles were wrong.

26. **The window hosts the run; it does not reimplement it.** `crate::lab` owns an
   `evo::Trainer` on its own thread and hands the window **one finished generation at a
   time** — the population snapshot, the eight showcase lanes with the exact seeds they
   were scored with, the whole history, the deaths, the recorded routes and the best-ever
   genome. The window animates that generation with `evo::EpisodeRunner`, and only when
   the animation ends does it ask for the next one, so the terminal mode's central
   property survives the move: the GA's RNG stream sees the same draws whether anyone is
   watching or not, and a **paused window trains nothing at all** (asserted on a real
   thread, see Testing). The thread exists for the same reason the packet does: a
   generation at pop 200 is tens of milliseconds across every core, and a window that
   blocks on it drops frames exactly while the user is trying to look at it.

   Three smaller decisions follow from the boundary rules. The chart is drawn as geometry
   rather than by calling `render::chart::fitness_chart`, which composes a *text* plot:
   `render` decides what the numbers mean (trend, stall, death cluster, the formatter) and
   the GUI decides where the lines go, which is the plan's own split — and it keeps the
   band the terminal's chart draws from drifting away from the one the window draws. The
   lanes are distinguished by outline colour and a bar at their feet rather than by eight
   glyphs, because a terminal is a character grid and a pixel view is not: eight sprite
   agents in one tile smear. And the mutation ladder moved into `evo` beside the
   hyperparameter it tunes (`evo::MUTATION_RATES`), because the terminal's `m`/`M` keys and
   the window's slider are the same control, and two copies of a ladder are two chances for
   the two front-ends to disagree about which rates are reachable.

   Inside the window, the same reasoning applies one level down: the panel's buttons, the
   sliders and the keys all produce `panel::Command` values, and one `apply` decides what
   each one means. The first draft of this view had the buttons' handling in one method and
   the keys' in another, which is two chances for "pause" to grow two meanings — and the
   mapping from pressed keys to commands is plain data (`panel::Keys` →
   `panel::commands_for`), so the part no test could otherwise reach is covered.

27. **Two bugs the new tests caught, both of the "looks fine, is wrong" kind.** The
   mutation ladder's nearest-rung search compared distances as *integers*, so 0.07, 0.05
   and 0.02 all truncated to zero, every rung looked equidistant, and the search returned
   the first one: pressing "up" at the top rate walked *back down* to the middle of the
   ladder. The behaviour was in the code that the terminal's keys reach through — but only
   the window's own ladder test pressed a rate that was already on a rung and stepped, and
   it failed immediately. The fix is `total_cmp` on the distances, and the test still
   asserts both ends.

   The second is a test-shape lesson. The new sprite-counting test asked for the
   *image-textured* vertices and compared a frame drawn with the editor's opaque options
   against one drawn with the play options plus agents: 492 versus 492, no agents — except
   the numbers matched for an accidental reason, because the editor frame drew one extra
   sprite (the spawn marker) and the run frame drew one agent. Tightening the assertion to
   `base + 4` — one agent is exactly one quad, and a level with no walkers is exactly one
   agent — is what made the frames comparable, and it now fails if a lane ever draws the
   whole population at once. Counting something is not the same as counting the right
   thing, which is finding 25's lesson one level up.

28. **The window hosts the run, not a convenient subset of it.** Finding 26 moved watch mode
   into the window; one feature stayed behind in the terminal — the stall fallback of finding
   22, "checkpoint the stuck population, train it somewhere easier, hand it back". It is now
   `evo::curriculum_step`, called by both front-ends, for the reason the mutation ladder
   moved there: a checkpoint and an excursion are properties of a *run*, and two copies of
   "save the population, train it elsewhere, adopt it back" are two chances for the two
   front-ends to disagree about what a stall costs. The window gained the terminal's flags
   with the terminal's meanings — `--resume`, `--curriculum`, `--curriculum-generations` —
   and both refuse the same way: a checkpoint that will not load is a startup error, and
   `--curriculum-generations 0` is a usage error, not a clamp.

   The subtlety is that resume and the fallback belong together, and only one of them was
   in the terminal. Without `--resume`, the checkpoint the window wrote would have been a
   one-way door: readable by `train-cli watch`, useless to the window that produced it.
   The pair is now closed in both directions, and the verification is the artifact rather
   than the screen: a window run stalls on L5, writes its checkpoint ("stalled: no new best
   in 25 generations (best 244.8 set at gen 9)", generation 35, two genomes), and a second
   window run told `--resume` on that file re-scores generation 35 and saves a champion
   whose generation is 35 and whose genome id is one of the checkpoint's own.

   Three smaller decisions follow from the same boundary rule. `f` is the terminal's
   follow/whole-level toggle, but the *defaults* differ because the media do: eighty
   columns cannot hold a whole level, so the terminal follows; this window can, so
   whole-level is what it shows and `f` is the viewer asking to zoom in on one agent. The
   rule for "the leader" — the live lane furthest along, counting a winner still standing
   on the flag — moved to `lab::leader_centre` so it is a tested function rather than a
   loop inside a frame, and the window then centres the camera on it (a *scrubbed* route
   centres on where the route stopped, which is the terminal's rule for the same moment).
   And the terminal's habit of saving the champion when a watched run ends has a window
   equivalent with one added restriction: the save happens when a **capped** run reaches
   its cap, because a cap is something the viewer asked for, so a `--screenshot
   --generations N` run leaves the artifacts a session at the keyboard would and an
   open-ended run needs the Save button it already has.

   The curriculum's sentence lives in `evo::CurriculumStep::note` and is *sticky* on the
   packet: "this run changed shape" is not something the viewer should have to catch inside
   a single generation, and the panel shows it for the rest of the run. That made one test
   interesting — "the run stepped back once" cannot be asserted by counting notes, because
   the note is meant to persist, so it is asserted twice over instead: the note never
   changes once it appears, and the checkpoint on disk is no newer than the generation the
   note first appeared at (a second step would have moved both). Two other tests were
   wrong in instructive ways. One pinned the literal `best there 812.35` for a value of
   812.345, which asserts a rounding rather than the note — `{:.2}` of that `f32` is
   812.34 — and now formats the expected string from the struct's own number. The other
   asserted that a run with nowhere to fall back to writes no checkpoint, using a shared
   scratch directory, so it found the checkpoint a *parallel* test had just written; each
   test's artifacts now live under their own tag. A test that shares a directory with
   another test is not testing the thing it names.

29. **The art pass: the tileset is now a set of rules rather than a set of pictures.**
   Four things changed together, and each one is a rule the code can be wrong about.

   * **Terrain is autotiled from the level, not chosen per tile.** `view::ground_mask`
     computes the 4-bit mask (open above / left / right / below) and asks the atlas for
     `ground_N` or `deep_N` by name, so a level's silhouette is *derived*: a floor, a
     wall and an overhang are three different sprites and no editor state is involved.
     `solid_at` deliberately disagrees with `Level::is_solid` about the level's own edge
     — the physics treats the border as a wall and the art treats it as air, and a
     leftmost floor tile would draw a sealed block if the two were ever confused.
   * **The picture is no longer a level floating on a checkerboard.** Play and evolve
     modes draw a sky gradient, two parallaxed hill bands and drifting clouds; only the
     editor keeps the graph paper, because only the editor is a *document*. Everything in
     the sky is hash-placed from an integer grid, so a level's skyline is the same on
     every frame and after every reload, and scenery (bushes, tufts, flowers, pebbles, a
     rock, a mushroom) is scattered on surface tiles by `view::tile_hash` for the same
     reason: a renderer that owns an RNG draws a different level every frame.
   * **Animation became state, and the state had to live somewhere.** `view::AgentAnim`
     is advanced once per drawn frame by the play session and by `lab::Lanes`, and it
     holds the three things a single frame cannot tell you: that a fall *ended* a moment
     ago (so the landing pose and its puff fire once), how long ago the last dust puff
     was, and how far the agent has run — the six-frame run cycle is driven by *distance*,
     not by wall-clock time, so a creeping agent's legs move slowly and a finished lane's
     dust settles instead of drifting on. Idle, landing, jump/fall and a death pose come
     from the sim; the mirror comes from the direction the agent was last heading, which
     is why a stopped agent keeps facing the way it ran.
   * **The art is checked as pixels, not as code.** The tileset's palette, layout and
     animation frames are asserted directly on the shipped PNG (finding 24's table grew),
     and three of those assertions failed the first time they ran and were each a real
     bug: a variant claimed to be "the tile plus detail" moved a row of the tile it
     varied, a *character* test discovered that the jump pose tucks its legs (so the
     airborne poses were separated from the grounded ones rather than the art being
     "fixed"), and the small rock stopped one row short of its cell's floor and would
     have floated a pixel above every surface it was drawn on.

30. **A walker could be placed, saved, counted and never drawn — because the only thing
   that could draw one needed a simulation.** `draw_agents` draws the walkers of a live
   `Sim`; the editor has no `Sim`, so `draw_level` was asked to draw the level's own
   entities and did not: goals and the spawn pad had their pass, walkers never got one.
   The document was right (the status line said "1 walker(s)"), the file was right, the
   tests were all green — and the canvas showed a floor with nothing standing on it, so
   the natural response was to place another one.

   Two things made it invisible to the suite. The editor's own test asserts the
   *document* (`walker_tiles()` after a paint), and the view tests counted terrain
   quads. Nothing asked the question "is the thing I placed on the screen?" The answer is
   now a test that asks exactly that: `headless_sprite_quads` returns every sprite quad a
   headless frame emitted with its atlas cell, and the walker test asserts one quad per
   placed walker, at the walker's own tile — the bug it describes reported *zero* quads
   for a level whose file had one.

   The fix splits the question the way the two callers actually differ. `RenderOpts`
   gained `entities`: the editor draws what the *file* says, and a live view must not,
   because a `Sim` marches its walkers away from the tiles they were placed on and draws
   them itself — a static pass underneath would leave a ghost of every walker standing
   where it started. Which *frame* of the march to draw is now `walker_frame(clock)`, a
   pure function of the clock that both a placement and a live walker ask, so the
   creature on the editor's canvas and the creature patrolling a run have the same legs.

   Verifying it on the level the report came from was worth more than the unit test: the
   author's own `l1-flat-run.json` now held five walkers, and the frame rendered from the
   app contains five walker sprites at their five tiles — feet on the floor line for the
   three standing on it, in mid-air for the two left floating at rows 6 and 7, which is
   what was drawn and is now what can be *seen*. Two tests that had quietly asserted
   things about that file's content rather than about the rule they named were fixed at
   the same time: the mask test now checks a tile's own neighbourhood instead of a row
   that is clean end to end, the lane count now asks the sim how many walkers an episode
   has, and the pose and run-cycle tests run on a runway they build.

   The same session also drew the tutorial level full of terrain, spikes and walkers it
   could not see, and *that* is the part worth keeping. The drawing was saved as
   `levels/scratch-l1-walkers.json` — it opens from the app's Open menu like any other
   level — `l1-flat-run.json` was restored from the original, and the pack's own rule is
   now enforced at the click: `Level::walker_stands_on_ground` is one predicate, asked by
   the editor before it writes a walker *and* by the shipped pack's test, so a walker that
   test would refuse is one the editor will not let you place. The drop ghost shows the
   answer before the button comes up (a red outline where the placement would be refused)
   and the status line gives the reason ("a walker needs ground under it"), because a
   click that does nothing and says nothing is a bug report.

31. **The run's answer needed a second view, and the window could not ghost a population it
   never received.** Watching evolution is watching a *search*: a generation at a time, at
   whatever speed makes a generation legible. When a viewer stops pushing that search, the
   thing they want is the opposite — the best agent the run found, playing the level once,
   at the speed the game actually runs at, with the population ghosted beside it. That is
   **Replay** (`V`, or `--replay`): one episode, 60 ticks a second, the champion solid and
   drawn complete, the field faint behind it.

   The champion and the field are deliberately **two different generations**, and the view
   says so: the champion is the run's best-ever agent, and the ghosts are the population of
   the last generation it scored — which is what the window holds and what `Packet`
   actually carries. On a run that has moved past its best idea they are different numbers:
   the 20-generation L3 run measured below has its record in generation 10 and its field in
   generation 20, and a 10-generation run at `--pop 300` has its record in generation 7 and
   its field in generation 10. So a stopped run is one long look at its best idea racing
   the population that has since moved on. Reading the field's generation off the champion
   would have been wrong in the ordinary case, so it is carried as its own fact
   (`replay::Population`) and printed in both the panel and the status line.

   The window could not do that with what it had. `Packet` carried the eight showcase
   lanes and the best-ever genome, and a genome is only *half* an episode: the seed is the
   other half, and neither the population nor the per-agent seeds reached the window at
   all. `Packet` now carries `agents` — every agent the last generation scored, each with
   the
   `EpisodeResult` it was scored on, built where both already existed (`step_verbose`'s
   population snapshot, index-aligned with the report's results). It is the same ~1.2 MB
   copy the lanes already pay for, once per generation rather than once per frame, and it
   makes the ghosts the population's *own runs* rather than fresh samples of its genomes.

   Four smaller decisions are the shape of the view. The champion's own genome is left out
   of the ghost roster: it is not *another* agent, and a second copy of the same network on
   a different seed would be a ghost that means nothing. Above `GHOST_LIMIT` (256) the
   roster keeps the best by the metric selection itself ranks on, so the cap is a statement
   about the run rather than about population order — and it is a parameter, because a cap
   a test cannot reach is a cap nobody has checked. Ghosts are drawn as **bodies only**
   (`view::draw_agent_body`, split out of `draw_agents`): a hundred agents sharing a level
   is the picture worth having, but a hundred copies of that level's walkers marching a
   hundred different distances is a curtain, and the dust, the landing puff and the flag
   belong to the one agent the replay is about. And hiding the ghosts is a *drawing*
   decision: every ghost keeps running, so bringing the field back shows where the champion
   actually has it rather than where it was when the viewer last looked.

   A fifth decision is what makes the field *legible* rather than merely present: it is
   drawn in **rank bands** (`GHOST_COHORTS`, four of them), not one flat tint. One colour
   says only "these are the others"; four say the thing a viewer actually wants to know —
   whether the agents clearing an obstacle are the ones the run thinks are good, or whether
   the stragglers are getting through by luck. The bands are proportional (`cohort_of`), so
   they fill whatever the run has — 200 agents make four bands of fifty, 7 make 2-2-2-1 —
   and they are drawn worst-first, so the agents nearest the champion's quality sit on top
   of the field rather than buried in it. The palette is chosen against the scene rather
   than in the abstract: the backdrop is a light blue sky over green hills, so the first
   thing a "cool" palette suggests — a pale blue-white body — is the one colour that
   vanishes against it, which is exactly what the first attempt drew. Cyan, violet, amber
   and salmon are hues the terrain does not use, and every band fades as the rank falls
   (an opaque ghost could hide the champion). The panel carries the legend for those
   colours in the canvas's own palette (`view::cohort_swatch`), and it draws the legend
   whether or not the field is showing, because a panel that reflowed every time `G` was
   pressed would move the controls under the viewer's hand.

   The replay opens on the **whole level**, not on the champion, which is the rule this
   window already documents for the watch view: `ViewState::follow` is off by default
   because a window can hold a level and an eighty-column terminal cannot (see the M3
   findings below). Following was the one place the replay contradicted that, and it is the
   place it matters most — a camera pinned to one agent hides the field the view exists to
   show, so "who else got through" was unanswerable from the picture rather than merely
   small. `F` follows the champion, `F` again gives the whole field back, and the camera is
   fitted to the **replay's** terrain rather than to the editor's document (`viewed_level`),
   because a run's level and an edit made since it started are two different levels, and the
   one that is not on screen is the one a wrong fit points at. That fix reaches the watch
   view too: fitting there now uses the run's level for the same reason.

   Entering the replay also parks the run, which is the design the whole window inherits —
   a run advances when the viewer has finished watching, never on a timer. Nothing is
   trained behind the replay, and going back with `Esc`/`V` resumes exactly where the run
   was.

   The claims are tests, not impressions. The champion's fidelity test runs the record's
   own seed and holds three numbers against it — the ending, the frame count and the
   fitness — because any other seed would move all three; the ghost test does the same for
   the whole field, one recorded result at a time; the clock test pins real time to the
   run's own cadence and documents the guarantee (a second is a second to within a tick at
   a frame rate the sim's step rate does not divide); and the toggle test compares every
   agent's frame-by-frame state with the ghosts on and off. The drawing itself is counted
   out of a headless frame — one quad per ghost plus the champion, one quad with the field
   hidden — which is the same question the editor's invisible walkers (finding 30) taught
   this suite to ask. The bands are held to their own properties: `cohort_of` never improves
   as rank falls and covers every position over rosters from 0 to 256 (with the band sizes
   pinned at 50-50-50-50 for 200 and 2-2-2-1 for 7), the ranges the panel prints are
   contiguous and cover the roster exactly, the four tints are distinct, translucent and
   monotone in opacity, and the *order* they reach the frame is asserted out of the
   headless mesh's vertex colours — worst band first, best band last, champion after it.

   Rendered from the app, the same run twice — once with the field, once with `--no-ghosts`
   (`--replay --generations 20`, at the default `--pop 200`, on L1 and on L3) — differs by
   4 209 pixels on L1 and 2 149 on L3, every one of them inside the canvas: the terrain,
   the backdrop, the chart and the panel's other rows are pixel-identical between the two
   frames, and outside the canvas the only differences are the two labels that report the
   toggle (the status line's ghost segment and the panel's checkbox — not the legend, which
   is drawn either way). Two runs of the same command are pixel-identical to each other, and
   saying that honestly took a fix: the window's scenery and the replay's *own* clock both
   ran on wall time, so a scripted frame was a race — the L1 frames of two runs of one
   command sat 391 pixels apart, all of them clouds. A `--screenshot` now holds the replay
   on the episode's clock, so the captured frame is that episode's 1.5 s mark every time;
   and a human who pauses the replay gets a still picture rather than a still field under
   moving scenery. With both frames fixed, the difference is the field and nothing else —
   which is also how the claim that ghosts do not perturb the agent being watched stops
   being an argument from code review.

   Those numbers also *moved* when the replay's opening view became the whole level, and
   they moved the way the renderer's own arithmetic says they should: a body is drawn
   `min(w, h) × zoom` pixels square, so a camera that drops from `FOLLOW_ZOOM` (34 px/tile)
   to the fitted ~17 halves every sprite — measured blobs 13–15 pixels wide before, 7–8
   after. Fitting is the more honest view as well as the one the view needs: at 34 px/tile
   the canvas held roughly half of a 60-tile level, so the field was cropped by the camera
   rather than merely small. (The field's own pixel total also grew when the player gained
   its outline — finding 32 — because a body with a rim is a bigger body.) The frames are
   in `out/`
   (`rp-l1-flat-run.png` against `rp-l1-flat-run-noghosts.png`, and the L3 pair beside
   them), and `--pop 300` renders the capped roster: 256 ghosts over four bands of 64, with
   the panel saying how many of the generation's 300 agents were left out.

32. **The player was the colour of the sky it ran in front of, and no test could see it.**
   The report was "I cannot see my own player", and the numbers agree: the runner was cyan
   `(92, 216, 234)` and the top of the sky is `(86, 160, 232)` — **56** apart in plain RGB
   distance, which is a camouflage jacket. It was the last place in the project where two
   facts that have to agree lived in files that never meet: the character's palette is in
   `tools/art/common.py` and the backdrop is in `crates/app/src/view.rs`, and the art
   tests — which do assert the *structure* of the shipped PNG, feet on the floor row,
   frames that differ, spikes that leave the ceiling open — had never asked what a sprite
   looks like against the thing behind it.

   The character is now a **vermilion runner with a cream scarf**, and its palette is
   chosen against the scene rather than in the abstract: vermilion is close to the
   complement of a light blue sky and of green grass, and the scarf stays out of the gold
   family that means "the goal". The body is **169** from the nearest backdrop colour
   (squared distance 3 176 → 28 628), and the walker (purple), the flag (red), the spikes
   (stone) and the goal (gold) all keep their own hues.

   The hue alone would still be a bet on where the character happens to stand, so every
   player frame is also **ringed**: one pixel of near-black around the finished figure.
   The rim is computed from the silhouette rather than drawn limb by limb
   (`characters._bump`), so every pose gets it — a trailing scarf, a raised knee, the
   crossed eyes of the hurt frame — and a change to the body moves the rim with it. It
   writes only to transparent pixels, so no detail is covered, and it is clipped by the
   cell, so the feet still stand on row 15 and the rim cannot sink into the ground. The
   replay's ghosts wear the same rim (they are the same sprite), which is also why the
   field's pixel total in finding 31 went up: a body with an edge is a bigger body, and
   the edge is what lets a viewer count the population over grass.

   Two pixel rules now hold it, in the art suite that reads the shipped PNG:
   `the_player_is_not_the_colour_of_the_sky_it_runs_in_front_of` takes the body as the
   modal colour *inside* the figure's edge and requires it to be at least 120 from every
   colour in `view::backdrop_colours()` — the renderer's own list, asked for rather than
   copied into the test, with the replaced cyan palette pinned in the same test so the rule
   cannot quietly stop being able to see the bug it was written for (it measures 3 176, and
   would fail). `every_player_frame_wears_one_dark_rim` wants an edge of more than twenty
   pixels, all one colour, at most half the body's luminance: an edge has to read as an
   edge rather than as a glow. Both frames are in `out/`: `player-art.html` puts every pose
   on sky, grass, hill and dirt either side of the change, and `player-ingame.html` is the
   champion in the real window, cyan then and vermilion now.

   Measured in that window rather than in the atlas — `--replay --generations 20` on L1 and
   on L3, at the opening view's fixed 1.5 s frame (`out/pv-l1.png`, `out/pv-l3.png`) — the
   champion is 6x8 px and the colour behind it is the pale hill `(126, 168, 186)`: **183**
   from the vermilion body and **224** from the rim. The two halves take turns there as
   well: `(55, 64, 80)` of deep rock is 201 from the body but only 60 from the rim, and
   `(160, 147, 135)` of dirt is 120 from the body and 196 from the rim. One case stays weak,
   and it is worth stating rather than rounding off: the level's own brown earth, where the
   body's darker shade `(186, 74, 30)` is **65** from the topsoil `(126, 84, 52)` and 89 from
   the deep dirt, with the rim 103 and 66 from those same two. That is the colour space
   refusing both halves at once — the lightest sky and the darkest earth pull a one-pixel rim
   in opposite directions, which is why the rim is dark (the sky is the common case) and the
   body is what carries the figure against rock. It is also why the first place to look if a
   falling agent still gets lost against a pit wall is the ground's interior rather than the
   character: a *cooler* earth band, not a brighter runner, is what would separate them.

33. **Every prop in the game was invisible, and the tests were all asking the right
    question about the wrong thing.** The scatter that dresses a floor — bushes, tufts,
    flowers, rocks, pebbles, mushrooms — has picked a sprite per tile since the pixel
    art arrived, and it has never once been seen: it drew the prop in the ground's *own*
    cell, and a solid tile is opaque by contract (`terrain.py` refuses a transparent
    pixel in one), so the terrain pass a few lines later painted over every one of them.
    The measurement is blunt: rendered off the same command, L1's frame contained **0**
    pixels of `LEAF`, `LEAF_LIGHT`, `MUSHROOM_CAP`, `PETAL` or `PETAL_PINK`, and 771
    after the fix. Nothing was wrong with the art or with the scatter: the tests on the
    scatter asked *which* sprite a tile wants, the test on the art asked how high up its
    own cell a prop's pixels sit, and neither of them asked which cell the sprite is
    painted in. `view::Prop` now carries the cell beside the sprite, and the cell is
    the **open tile above** the surface (for a vine, the open tile below the solid one),
    so "a prop nobody can see" is no longer expressible at the call site.

    The dressing grew at the same time, from 17 props to 25 (the atlas is 79 sprites of
    96 cells): `log` and `stump` for a floor with history on it, `reeds_a`/`reeds_b` for
    the tallest thing that grows out of one, `flowers_white` for a third flower, and
    `mushroom_pair`; `vine_a`/`vine_b` are the first *hanging* scenery, drawn under a
    solid tile with open air below it — one column in six, because a vine per column is
    a curtain. The eight new names are **optional art**: each has a fallback the view
    falls back to (a log a tileset lacks is a rock, reeds are a tuft, white daisies are
    the gold ones), so a tileset drawn before them still runs dressed rather than
    refusing to start. Four rules hold it, all over the shipped PNG: the props stand on
    row 15 of their cell and the vines touch row 0 of theirs (an anchored or hanging
    sprite that stops a pixel short reads as floating); every prop the scatter can name
    is really drawn somewhere on a long probe floor, because a name that never resolves
    draws its fallback for ever and nobody can see the difference; the scatter dresses
    between 15% and 40% of the floors it could (0.266 on the shipped pack: 94 props over
    353 surface tiles) rather than paving them; and a vine only hangs where the cell
    above is solid and the cell itself is empty *and in the level*, so none dangles under
    the world's bottom edge and none grows through a spike.

    `assets/README.md` was wrong in the same way and for the same reason: it still
    documented an 8x3 atlas of `grass`, `dirt` and `player_stand` long after the tileset
    had seventy names, because prose about a generated file is prose nothing checks. It
    now lists every shipped sprite, and
    `atlas::tests::the_assets_readme_lists_every_shipped_sprite` reads the manifest and
    the README and fails when one names a sprite the other does not. Frames: the new
    scenery in a real window in `out/scenery-check.html` (vines under two platforms,
    reeds and flowers on top of one), the eight new cells at 8x in `out/props-zoom.html`.

34. **The frame was drawn at a fractional scale, and the sky had two distances where it
    wanted three.** Both are the kind of thing a still screenshot never shows. A tile at
    17.2 points is seventeen pixels wide on one row and eighteen on the next, and a solid
    tile's rectangle used to be rounded *on its own* — so two neighbouring floor tiles
    could leave a one-pixel seam or overlap by one, depending on which way each one's
    fraction fell. The camera now has one integer `scale()`, one origin snapped to whole
    pixels, and every rectangle derived from them: `tile_rect` is exactly `scale()` across
    and shares its edge with the next tile **exactly**, `world_rect` rounds the *size* so an
    agent's body cannot gain and lose a pixel of width as it walks, `zoom_at` rounds and
    `fit` floors so the fitted view still fits. `the_camera_lands_on_whole_pixels` pins the
    arithmetic at zooms from 6 to 96, including a fractional one, and checks the inverse
    property (`to_tile` ∘ `to_screen`) the editor's brush depends on.

    Three smaller things fell out of it: the status bar printed `zoom {:.0} px/tile` from
    the raw zoom, which is not the zoom the frame is drawn at; panning divided the drag by
    the raw zoom, so the level moved slightly further than the grid under the cursor; and
    the editor's `tile (x, y)` readout divided the *screen* pointer by the zoom, which is
    the right tile only while the camera sits at the level's origin. The first two now ask
    the camera for its scale, and the third asks the camera to invert the point — the
    readout cannot disagree with the grid because it is the same function that drew it.

    The sky grew the layer it was missing. `SUN`, a colour in `tools/art/common.py` since
    the first commit, had never been drawn by anything: it is now the sun — a disc with two
    dimmer rings at the smallest parallax in the scene (0.04), so half a screen of walking
    moves it four pixels.    In front of the far hills stands a **tree-line** of two new
    conifer silhouettes (the pines, 83 sprites of 96 cells now) at a parallax
    between the two hill bands, planted on the far hills' horizon line — sparse on purpose,
    two columns in three left as sky, because a solid line of trees reads as a fence. That
    anchor, that parallax and that spacing were all wrong, and finding 35 is what it took to
    see it. The
    sun and the pines went into `view::backdrop_colours()`, so the rule that holds the
    player's palette (finding 32) now covers the pale disc and the dark silhouette as well
    as the sky: a character chosen against four colours and drawn in front of six is the
    same bug with two more ways to happen.

    Two new tests hold it, and the harness under them is the useful part: a headless frame
    is now read back **quad by quad** (`view::tests::Frame`) — screen rectangle and atlas
    rectangle per sprite, plus every vertex colour for the shapes — so a test can ask which
    sprites a frame actually drew. That is the question nothing in this project could ask
    until now, and it is the one that would have caught finding 33 on the day it arrived.
    Frames: `out/sky-check.html` (L1 whole-level, and the overhang demo at native scale,
    both with the new sky), `out/scenery-check.html` for the dressing.

35. **The tree-line was hovering in the air, the trunks were in front of the foliage, and
    the trees were the wrong distance away.** A player reported all three, in that order, and
    the first turned out to be two bugs stacked.

    The pines were planted on `SKY_HORIZON` — the line the far hills' *base* meets the sky
    — on the theory that a band's horizon is where a band's things stand. It is not: the
    domes rise above that line, and at the columns the trees occupied the band's visible
    silhouette was **33 to 76 points higher**. So every pine was drawn inside the hill's
    flat fill, with hill colour above its crown and below its trunk, and the nearest thing
    below *that* was the level's floor another 22 points down: 14 trees hovering in front
    of a hill. Underneath sat a second one — `draw_hills` drew a wall *per dome*, so a dome
    the hash skipped took its wall with it, and one column in five was a stripe of sky from
    the top of the window to the floor. One tree in thirteen stood in one of those, in open
    air.

    Both are now one object. `HillBand` builds the geometry once — a single base across the
    whole span, then domes on top of it — and answers `ridge(x)`, the silhouette a thing can
    stand on, so the painter and the planter cannot disagree about where the ground is. The
    domes are drawn as a hand-built fan rather than `circle_filled`, because the ridge has to
    be *that* geometry: egui feathers what it paints, filling half a point inside the shape,
    and at a dome's rim half a point along the normal is two and a half points straight down
    — a sliver of sky under a tree whose foot was measured against the circle. A hard-edged
    fan has no such gap, and pixel art wanted a hard edge anyway.

    The tree-line also moved into the far hills' layer. It had its own parallax, which reads
    as a third depth until you notice that a tree standing on a ridge slides along it: it
    crossed a dome on every step of the camera, its foot climbing and falling with the
    silhouette it was supposedly planted on. Two things that stand on each other are one
    layer. The scatter, the two shapes and the whole-pixel sizes are unchanged; each tree's
    foot is now `ridge` at its own centre column, rounded up so it is planted **on** the line
    rather than a hair above it.

    The second report was a different kind of bug — the art's, not the scene's. `pine()` drew
    the trunk *last*, so it was a dark bar laid across the lower branches: foliage hanging
    off a post. The trunk now goes down first and the skirts over it, which leaves the bare
    stem showing under the lowest skirt — the only part of a distant conifer's trunk an eye
    can see — and the lowest tier moved five rows up so that stem is not one pixel of it.

    Verified on the frame, not the intent. The test harness now reads a frame back as
    **triangles** (`view::tests::Frame::triangles`), which answers the question a sprite's
    placement actually needs — *what is drawn at this column* — as point-in-triangle tests on
    the geometry itself, rather than by fitting circles to runs of vertices. Fitting was tried
    first and abandoned: a circumcircle through three points a fraction of a pixel apart is a
    point out, which at a dome's rim is five. `every_pine_stands_on_the_hills_ridge` asserts
    each pine's foot is on the drawn ridge to within its own whole pixel, and that the band is
    under it; `the_hill_bands_have_no_hole_in_them` samples every five points across the window
    and asks whether either band still covers its own horizon. Both were checked against the
    old code before being trusted: restoring the horizon-line anchor fails the first one
    immediately. On the rendered frame (L1, `--evolve`, 1360x800) all **14 of 14** trunks have
    the far band's first row directly under them — gap zero — where the same probe before the
    fix put every one of them 33 to 76 points inside a dome.

    The third and fourth reports were the ones no geometry test could have caught, because the
    geometry was right: the pines were drawn **too dark and too big**, which is the same
    mistake twice — a treeline three hills away, painted in the tones and at the scale of
    something in the foreground. Aerial perspective is not decoration; it is the depth cue the
    parallax multiplies, and both of these spent it on the wrong side.

    The tones first. Each of the four is now the conifer green mixed **62%** of the way to the
    far hills' own colour as the renderer draws them (`view::HILL_FAR`, 126, 168, 186) — the body
    went **101.5 → 50.8 → 39.1** away from the hills across two attempts, and the darkest tone,
    the trunk, **160.9 → 64.2**. The four are spread around that base by fixed steps rather than
    mixed by the same fraction, because a haze that flattens the *shading* as much as the hue
    leaves a blob where a conifer's tiers should be; the trunk stays the darkest thing in the
    tree-line, since it is the part that has to read against the hill it stands on. The rule that
    holds the player's palette (finding 32) now carries all four pine tones
    (`view::backdrop_colours`, nine colours) instead of one, so it covers every colour a runner
    can be drawn against rather than the middle of them.

    Then the size. A tree was drawn one and seven tenths to two and eight tenths **tiles** tall,
    against hills three and a half to five and a half tiles across: a pine as tall as half the
    hill it stands on reads as a row of trees planted just behind the level's back wall. It is
    now **one to one and a half tiles** at every zoom — 0.7 to 1.2 tiles of visible silhouette,
    since the sprite's own margins are transparent — and the spacing came in with it, 3.4 tiles
    to 2.6, because smaller trees at the old spacing stop reading as a line. That is the part the
    test pins now: `every_pine_stands_on_the_hills_ridge` asserts a pine is between one and one
    and a half tiles tall on screen, so a regression to the old size fails a test rather than
    waiting for someone to say it looks close again.

    The fifth report was the one the first four were all circling, and it was not about a tone or
    a size at all: **the line did not look like a forest edge, it looked like a row of evenly
    spaced single trees.** Distance properties had been tuned for four rounds — a colour, a
    height, a planting depth, each a single number — and the thing that was actually wrong was the
    *shape* of the line, which none of those numbers touch. Twice over, as it turned out.

    The spacing first: the two columns in three left as sky were described above as "sparse on
    purpose, because a solid line of trees reads as a fence". True, and it produced the opposite
    failure, because that spacing was the *only* structure the line had — one tree per position,
    every position the same distance from the next, and the eye locks onto that rhythm in three
    trees and never lets go. The line is now **clumped**: a position plants two or three trees
    packed half a tree's own width apart, so that within a clump they are growing out of one
    another, and one position in three is left as sky. Bunched clumps with clearings between them
    — not singles with gaps, which is what even spacing *is*.

    The clumping was tried once before and missed, which is worth recording: that version planted
    its clump at the *rank's* spacing, twenty points apart at a tree size of twenty-five, so two
    trees called a clump stood three quarters of a tree of sky apart and read as two trees. What
    the packing is a fraction of is the fix — a tree's own width, not the interval between clumps.

    Then the depth. A second rank now stands **behind** the first: the same two silhouettes in the
    two-tone haze of `common.PINE_FAR_*`, a size smaller, planted more densely (one position in
    four left bare, against one in three) and buried a third of a tile into the ridge, so its
    trunks cross the skyline and its bases are hidden on the hill's far slope. Denser than the rank
    in front on purpose: what the front rank's clearings must show is *more trees*, and a clearing
    in both ranks is a hole in a fence. That is also why the two ranks are two palettes and not two
    sizes of one — a forest edge is a mass with a ragged top, and tone is how a flat picture says
    which part of it is further away.

    The geometry had been tested since finding 33 and the shape never was, so it is tested now:
    `the_tree_line_is_bunched_with_clearings_between` asserts that **most** neighbouring pairs in
    the front rank are closer than forty per cent of the rank's own interval (a clump, and not one
    pair of neighbouring clumps can fall under it), that the bald stretches between clumps are at
    least two tiles wide, that most of those clearings have a tree of the rank behind them, and
    that every tree of the far rank reaches the frame **before** the first tree of the front rank
    does — draw order is depth, so a rank that drifted in front of the one it stands behind would
    fail a test rather than show up as trunks crossing the wrong hill. The thresholds are measured,
    not guessed: the line as it was bunched eleven of thirty-four pairs (a third) and the line as
    it is bunches twenty-three of thirty-eight (three fifths), so "most" sits between two numbers
    that were read off the code rather than chosen. The line as it stood — the old count and the
    old spacing, restored together — was run against the test before the test was trusted, and it
    fails it: eleven of thirty-four pairs.

    On the rendered frame (L1, `--evolve`, 1360x800) the front rank went from 29 clusters to 18
    while its median cluster width went 17 points to 24, and the rank behind from 367 covered
    columns to 475: fewer, wider clumps in front of a wider band behind.

    `assets/README.md` had the tree-line "standing on its horizon" too, and now says ridge.
    The atlas is otherwise the same 83 sprites of 96 cells; the pines are the only art that
    changed.

36. **The foreground grew a second material, everything bigger than a tile it had, and a
    bug the tests could not see: the flowers had been paid for and never drawn.**

    The atlas went from 96 cells to 144 (16x9) and from 85 names to 125 — forty new ones.
    **Sand** is eighteen of them: the sixteen autotile masks in a second material
    (`sand_0..sand_15`, a bright crown of loose sand over packed sand) plus two decorated
    variants, `sand_1_b` (wind ripples) and `sand_1_c` (a shell and a scatter of grit), the
    same bargain the lawn's own variants make. Under a beach is **sandstone** (8), the same
    masks with no sky above them, drawn with horizontal bedding planes so it cannot be
    mistaken for the vertical-cracked stone under a lawn. Then **four trees**: `tree_a_base`
    and `tree_a_top` (and a second shape, `tree_b_*`), each drawn whole and cut into two
    cells at a seam the halves agree on column for column — the one thing a two-cell prop
    can get wrong that a one-cell prop cannot. Two **bushes** (`bush_e`/`bush_f`: half the
    height of the others and twice as broad, so the shape survives a one-tile ceiling) and
    four **grasses** (`tuft_c`/`tuft_d` long dry blades gone to straw, `tuft_e`/`tuft_f`
    short dense turf) round out the dressing, and four **butterflies**
    (`butterfly_a_1`/`_2`, `butterfly_b_1`/`_2`) are the only sprites in the game that
    stand on nothing at all.

    **Sand is a material, not a mechanic.** `S` in the level format: solid in every
    direction exactly as `#` is, and the same tile to the physics, the sensors and the
    reachability oracle — the sim, `sensors::encode_tile` and the agent's 9x9 patch cannot
    tell the two apart. What the claim is worth is what the test does: the same level twice,
    floor written `#` then `S`, one seeded action stream of four hundred ticks, and the two
    simulations must agree on the outcome *and* the state hash they end on. The glyph
    round-trips, so opening and saving a level cannot turn a beach into a lawn. Only the
    view differs, and it differs in one place: `autotile` returns a pair — the material's
    name and the soil name to fall back to — so a tileset drawn before sand existed draws a
    beach as soil rather than refusing to start (the test deletes the `sand_*` names from
    the manifest and requires `ground_1`).

    **The bug was in the roll, and no test could have caught it where it was.** The scatter
    picks a prop per surface tile from `tile_hash(tx, ty, 23) % 100`. A floor is a
    horizontal run of tiles at one `ty`, and the *low* bits of that hash move in step along
    a run: over the shipped pack's hundred distinct surface tiles, `% 100` never once lands
    in 15..=17, which is the flowers' band. Six levels, eight flower slots paid for out of the
    table over two distinct sites, and not one daisy drawn anywhere in the game. Neither test that existed
    could see it — the probe-floor test asks whether a name is *reachable* (it is, over a
    synthetic four-hundred-tile floor) and the density test only counts — and the same
    blindness is what finding 33 was about, one layer down. The roll is now the top slice,
    `(tile_hash(..) >> 18) % 100`: the mixed bits, 68 of the hundred buckets reached over the
    same pack where the low slice reached 59. The new test that would have caught it reads
    the six shipped levels and requires **every prop family the table can name to be dressed
    onto them** (`every_prop_family_is_dressed_onto_the_shipped_pack`); restored to the old
    roll it fails with "no flowers is drawn anywhere in the six shipped levels", naming the
    nine families that did appear. One honest corner of that: the pack is a hundred distinct
    surface tiles, so it holds two flower *sites*, and both of them roll the pink daisies —
    the gold and white ones are reachable (the probe floor, which has four hundred) but a
    three-way split of two tiles cannot show three colours, and a test that asked the pack for
    all three would be asking for luck rather than for correctness. On the fixed roll the pack is 127 props over 353 surface
    tiles — 0.360, up from 0.266, because ranges that were unreachable are furniture now —
    which is inside the dressing band the density test has always asserted (0.15..0.40).

    The dressing itself is two rules richer. **A tree needs room**: every cell its canopy
    reaches into above the ground it is planted on — which is three cells at the height it is
    drawn at now (finding 39) and was two while it was a two-tile tree — and they have to be
    *empty* rather than merely not solid, because a tree growing through a spike is a
    decoration lying about a hazard. Where there is no room it is a bush — never a trunk with
    its canopy inside the ceiling, which is a tree growing through rock — and the test proves
    the rule by building the same floor under ceilings one cell higher each time, a corridor, a
    two-cell slot and the height a tree is really drawn at, and requiring trees only at the
    last one. Over the pack eighteen trees are drawn and no slot was short of room, so the
    fixture is what holds the rule. **Butterflies are not scenery**: they belong to no tile, so
    they are a layer of their own behind `RenderOpts::ambient` — one per fifteen tiles of
    standing room, drifting an ellipse 0.16 of a tile across, wingbeats at 7 Hz off the same
    clock as everything else, and drawn only where the level calls the tile empty, the tile
    above is open and the tile *below* is solid, so no butterfly is ever inside a wall, squeezed
    into a one-tile gap between two platforms, or up in the sky with the birds (finding 40).
    The pack holds twenty of them; L1's opening frame holds three, and the editor holds none,
    because a document is about what the file says.

    The refactor underneath was checked against the old sheet rather than trusted: making
    "a material" a value instead of three copies of the autotiler left the twenty-five soil
    and rock cells (`ground_0..15`, `deep_0..14`) **byte-identical** to the atlas drawn before
    it, compared cell by cell against the older sheet in `out/`. What did change is what was
    asked for. `assets/README.md` grew the new names and the material paragraph, the ASCII
    legend grew `S`, and the level-format section names the glyph. Frames: the beach demo in
    `out/sand-beach.html` — `levels/scratch-sand-beach.json`, a sand beach meeting a lawn on a
    flat floor, a four-tile dune over sandstone, and a raised sand shelf — where the sand
    family is 16 888 pixels of the game's own view (1.55% of the frame, 15 157 of them loose
    sand and 1 731 sandstone) and 10 342 in the editor. That level also answers the question a
    screenshot cannot: `--evolve --pop 8` wins it (best_x 57.1, 0.0 tiles from the flag) and
    the editor and `train-cli reach` both call it beatable over five platforms, so the beach is
    drawn as sand and walked on as ground.

37. **Two materials meeting looked like a line, because it was one: the blend is a
    property of the material, not of the pair.**

    A beach that begins in the middle of a lawn used to share an edge with it: two flat
    columns of pixels, a sand crown ending exactly where a grass crown starts, and the eye
    reads that as a boundary drawn with a ruler. Now each material creeps a few ragged
    pixels into the tile beside it — the *neighbouring* material's band is drawn over the
    tile at the seam, so the lawn puts grass on the beach's edge and the beach puts sand on
    the lawn's, and the two crossing is the transition.

    The bands are per **material and side**, never per pair of materials: `blend_beach_l`,
    `blend_dirt_b`, and twenty more of them. That is the decision the whole thing turns on. A
    table of *pairs* would need each band once per ordered pair of materials — the same sand
    band drawn again for lawn-into-beach and beach-into-lawn — so a third material would
    multiply the art instead of adding four cells to it, and every one of those sprites would
    have to be drawn, reviewed and kept consistent. Six *looks* — the two materials' three
    appearances each, `grass`/`dirt`/`stone` for soil and `beach`/`packed`/`sandstone` for sand
    — make twenty-two bands covering every seam in both directions. Two of the twenty-four are
    structurally impossible and are not shipped: a surface tile has sky above it, so no solid
    tile ever has a crown *below* it, and `blend_grass_b` and `blend_beach_b` describe a level
    the format cannot build.

    A band is not painted, it is *cut*. It is the material's own plain interior tile with
    everything but a ragged edge band made transparent, so the pixels of a seam are the
    material's own pixels: `a_seam_is_made_of_the_materials_own_pixels` reads the shipped PNG
    and requires every opaque pixel of every band to be a colour of the tile that look is
    drawn from. That is what makes a seam impossible to get subtly wrong — there is no second
    palette in which to choose a nearly-right yellow — and it is also why the band contains
    the crown at its top rows and the body below them, from the same cell the material is
    drawn with: at a vertical seam the grass *line* carries on into the sand, green at the
    surface and brown under it, without a single tone being named twice.

    **The grains are the second half of the fix, and the first half was the bug.** A band used
    to be opaque along its own edge, on the theory — written down here — that the two materials
    have to *meet*, and that a band which dithered to its own edge would leave the sky showing
    through the floor. They do meet, but not there: a band is drawn in the tile it belongs to,
    so both of them covered their own tile line and the pair stacked into `A B | A B` — a stripe
    of the neighbour inside each tile, two hard parallel lines a pixel apart, and an eye reads
    *both* of them. Sand against soil, whose tones are five steps apart, is a bright line with a
    dark one beside it for the whole depth of the ground: a seam drawn with a ruler under a new
    name, and it was reported as one. No sky was ever at risk, either — a band is drawn *over*
    the tile's own opaque cell, so every pixel of a seam belongs to one material or the other
    whatever the band says — which means the old rule was protecting against nothing and costing
    the whole effect.

    So a band is a **scatter of grains** now: a row of the seam reaches nothing, one pixel, or
    all the way in, the grains inside a row it does reach thin out with depth, and the boundary
    between the two materials therefore wanders a few pixels either way with grains of each on
    the far side of it. How far a material reaches is a property of the material rather than of
    the pair — over the shipped atlas, sand reaches four pixels, soil two, stone one, sandstone
    two — and `every_seam_is_a_band_along_its_own_side` asks four things of every band instead
    of three: mostly transparent (the largest is 18% of its cell), never reaching the far side,
    carrying its material across *most* of its own edge but not the whole of it, and thinner at
    its deepest than at its edge. The renderer's side of the contract is one rule, and it
    is the one the tests read back off the frame rather than trusting: for each of a tile's
    four sides, if the neighbour is a *different material*, the neighbour's band is drawn in
    this tile's cell and nowhere else. `a_seam_carries_the_material_across_the_edge_it_lies_on`
    checks that invariant on a probe floor built to contain every seam the format can make —
    it draws the probe, reads the drawn rectangles back through the camera's inverse, and
    requires each one to sit on the tile whose material differs from the band's, with the
    band's own material in the tile across the edge — and it counts them: the number of bands
    in a frame is the number of material edges in the level, so a seam costs one sprite and a
    level of one material costs none (`a_floor_of_one_material_has_no_seams`).

    The one thing the blending deliberately does *not* do: two looks of one material do not
    blend. Grass over the dirt beneath it is a crown and a body, not a seam, and the crown's
    last row has been drawn ragged against the body since the material was generalised for
    exactly that reason — finding 36's crown is what makes the surface line a *line*, so
    blending there would smear a boundary that is deliberately exact. That is what the family
    check in `draw_seams` is for, and why the two-row floor of a shipped level draws nothing at
    all when nothing else is next to it.

    The atlas is 147 sprites of 160 cells (16x10 — the seams are their own family at the end of
    it, so the 125 sprites that were there before them kept the cell they had and the diff to a
    generated file stayed additive; every one of those 125 cells is byte-identical). Frame:
    `out/seam-zoom.html` — `levels/scratch-seam.json`, a lawn meeting a beach on a flat floor
    with a sand shelf standing on the lawn and a sand step on the beach, at 4x, which is where
    the ragged few pixels at the seam are a thing you can see rather than a thing this text
    claims. The grains have a frame of their own and it is a *pair*: `out/seam-before-after.png`
    renders the same lawn|beach seam at 9x twice, once through the old band code and once through
    the new one, so the two hard parallel lines and the ragged interlock are the same pixels in
    the same order rather than two screenshots from two afternoons.

38. **The sky was painted on the window, not on the world: every layer hung off a fraction of
    the viewport, so the hills and the clouds sat correctly in exactly one camera.**

    A player reported the hills standing in the air outside the level, the clouds and the sun
    with them, and gaps between the tops of the near hills and their own ground. The first is
    one decision and the second is one arithmetic slip, and both are the same mistake about
    what the sky *is*.

    The layer positions were `0.62` and `0.78` of the viewport's height for the two hill bands,
    `0.10` for the sun and `0.05 + 0.07·row` for the clouds — a landscape described in window
    coordinates. That is not a coincidence to be tuned away; it is one camera looking at one
    level, written down as four numbers, and it stops being true the moment anything else is.
    A follow camera puts the level's floor somewhere else on the screen, a taller level's floor
    is somewhere else again, and a level whose ground is not its bottom two rows is somewhere
    else a third time. In the fitted view the level occupies the middle third of the canvas and
    the ground plane at 0.78 of the window sits *below* it: the backdrop then crosses the
    level's own air, the hills stand in the air with it, and the level reads as a document
    floating over a landscape painted for a different frame — which is what the screenshots
    showed.

    The arithmetic slip is smaller and was visible in every frame. The old band was a base
    rectangle across the whole span with one fan per dome sitting on it: correct where a dome
    covers a column, and the flat base's top edge everywhere else. The far band's domes were three
    and a half to six and three tenths tiles across in a six-tile spacing, so its neighbours
    always met; the near band's were two and seven tenths to four and eight tenths in the *same*
    spacing, and the hash skips one dome in five — so between the two neighbours of a skipped
    dome the rims stand twelve tiles apart and reach at most four and eight tenths each, which is
    a flat stripe of hill colour at least two and a half tiles wide every fifth position, with the
    range behind it above and open sky beside it. Both reports are the same bug at two distances.

    **Everything is placed off the level's own ground line now.** `sky_ground(level)` is the top
    of the deepest run of solid rows — the surface the level is walked on, which is neither the
    bottom row's own top nor the level's bottom edge — and `sky_line` puts it through the camera,
    so the whole sky is a picture of the world at the world's own scale: the two bands' domes are
    3.4 to 4.8 and 2.5 to 3.2 tiles tall, the sun stands 10 tiles over the ground and three rows
    of cloud drift at 5.5, 7.0 and 8.5. Tiles, not pixels and not fractions: every zoom shows the
    same hills on the same line, and the ground plane beside a level meets the level's own floor,
    which is the thing that stops it being a plateau standing on the world. The clouds moved
    *behind* the hills, where they were never tested before — pinned to the top of the window they
    could not overlap a hill, and at their own height in the sky they do, and a cloud a hill can
    stand in front of is a cloud that is further away than the hill rather than a cut-out pasted
    over it.

    **A band is one closed silhouette, and its top edge is the ridge function itself.** Sampling
    the very line [`view::HillBand::ridge`] answers "what is under this column" with across the
    span means there is no base rectangle to show: between two domes is the band's own ground, and
    a `SWELL` of 0.35 to 1.1 tiles — a slow wave on the band's own coordinate, so it rides the
    parallax — keeps that ground off the horizon everywhere, which is what a gap in a range is
    made of. The shortest dome in a band is at least half the spacing, so two neighbours always
    meet; a dome the hash leaves out is a saddle rather than a hole, which is what "one in five is
    a gap" was. The band is drawn eight tiles past the window on each side (`SKY_PAD`) because the
    tree-line plants its clumps outside the frame, and a band that stopped at the edge left those
    trees in the air off it. The silhouette is left fractional rather than snapped to whole points
    like the rest of the frame: a flat fill's edge is the one place a fractional coordinate costs
    nothing, and a staircase of whole points puts a tree planted on the ridge beside the edge it
    was planted on.

    Four tests hold it, and they are the ones that were missing rather than the ones that were
    wrong. `the_hills_stand_on_the_levels_own_ground` asks each band for its height over the
    ground line in *three* cameras — the fitted view, a follow camera at 28 points a tile and a
    zoomed-in one at 72 — and requires it to be the band's own height in all of them. The old
    geometry could not pass it in *any* camera: the test asks each band for its height over the
    level's ground line, and a fraction of the viewport is the same number in every camera, so it
    is the right number in at most one of them. `the_hill_bands_have_no_hole_in_them`
    requires the silhouette to be *above* the horizon at every column (the flat stripe, from the
    drawing side) and the band solid under the ground line; `the_sun_and_the_clouds_are_in_the_levels_sky`
    checks the sun at exactly its altitude and then moves the camera five tiles and requires the
    sun and every cloud to move five tiles with it, which is the difference between a world and a
    pane of glass; and `the_ground_line_is_the_top_of_the_levels_floor` pins the line itself over
    four floor shapes — one row, two rows, a floor with a pit in it, and a level with no ground at
    all, where the level's bottom edge has to do — plus the six shipped levels by *property*: every
    column of the ground line stands on ground with air above it.

    What it costs and what it does not do. A band is one mesh of two triangles a point across its
    span: 2 248 triangles at 18 points a tile and 2 696 at 28, so both bands are 4 500 to 5 400 of
    the frame. And the sky is now the level's: a follow camera at 28 points a tile shows about
    twelve tiles above the ground, so a level's sun and clouds sit *in the level's own air* rather
    than in a band of window above it, and a camera zoomed past that takes the sun out of the
    frame — which is what looking closely at the ground does. In the fitted view there is empty
    sky above the level's top row, and that is the world's sky: it belongs to no level. Frames:
    `out/backdrop-frames.html` plots the drawn geometry — every triangle and sprite rectangle — of
    a fitted and a following camera side by side, which is how the two reports were confirmed and
    how the fix was read back; `out/sky-after.html` is the same level in the game's own view.

39. **Everything on a lawn was the same size, and the tree was one of the small things.**

    A player reported the foreground trees looking *very small next to the flowers*. Measured,
    the report is exact, and the reason is that the view had no scale in it anywhere: every prop
    in the game was drawn at exactly one cell — `sheet.image(sheet.tile(..), sprite)` and not a
    size in sight — and a cell is about the height of an agent. What a lawn therefore had on it,
    in tiles of its own ink, was a bush 0.63 across, a patch of daisies 0.56, a mushroom 0.81 tall
    against a stump's 0.88, and one tree: two cells at one tile each, canopy and all, so the only
    thing on the floor that is supposed to be big was two tiles tall with a canopy one cell wide
    — a shrub on a stick beside a mushroom it did not dwarf. Nothing was wrong with the art and
    nothing was wrong in a line of code: what was missing was a number saying how big each kind
    is, which is exactly the kind of thing every test the view had was blind to, because "which
    sprite does this tile want" cannot see the size of the answer.

    **A prop's cell is a slot now, not a size.** `Prop` carries the height it is drawn at in
    tiles, one table (`view::prop_size`) gives every kind its own, and `draw_prop` puts the
    bottom of the rectangle on the bottom of the cell and measures the art up from there. That
    one rule covers both directions at once, which is why it is one rule: a tree at `TREE_SIZE`
    grows past the cell it stands in, a daisy patch at a third of a tile stands *on the ground*
    inside a cell it does not fill, and neither of them has to be told which case it is. The
    sizes: trees 1.5 tiles per cell of their two, bushes 0.75, tufts 0.7, daisies 0.6, mushrooms
    0.7, pebbles 0.75, rocks 0.85, logs and stumps 0.9, reeds and vines their full cell. On the
    L4 rhythm the tree went from 24 points tall at the fitted zoom to 36 — three tiles against
    the daisy patch beside it at 7 points, 0.58 of a tile — so the thing that looks like a tree is
    now four times the height of a bush, four times a tuft of grass and five times a patch of
    daisies, where all of them used to be one cell and the tree's canopy was one cell wide.

    Two consequences worth following through rather than discovering later. **The canopy is
    placed off the trunk and not off the cell above it** — a tree at 1.5 tiles a cell stands
    three tiles and the cell-relative arithmetic that was right for a one-tile prop puts a stripe
    of sky through the crown — so `draw_prop` lifts the crown's rectangle by exactly the height
    the trunk was drawn at, and `a_tree_is_planted_as_two_cells_that_meet` now asks the two halves
    to *touch* and the pair to be at least three tiles tall, instead of asking the canopy to be
    one cell above the trunk. **And the room a tree needs is the height it is drawn at**, which
    is now three cells rather than two: a two-tile tree fitted under a ceiling a two-tile tree
    fits under, and drawing it through the rock above would be the same lie the rule has always
    been there to stop. `a_tree_is_a_bush_where_there_is_no_room_for_a_canopy` builds the same
    floor under ceilings one cell higher each time — a corridor, a two-cell slot, and the height
    a tree is really drawn at — and requires trees only at the last one, which is the case that
    changed.

    What is *not* claimed: the art did not change, so a tree is still a 16-pixel cell blown up to
    twenty-one or twenty-two points, and its pixels are half again the ground's rather than 1:1. That is the same bargain the pines have always made — a 16-pixel silhouette drawn up to
    1.6 tiles — and the alternative was a redraw of four cells into a 2x2 block; what would be
    wrong is claiming a tree was drawn big when it was drawn small. The frame is
    `out/scenery-after.html`: the size rule itself (one cell per prop, then each kind at its own
    height, on a common ground line), plus L4's tree-daisy-tree row, the small dressing, a tree on
    a beach shelf, and the three tests that pin the ordering, the ground line and the tree's own
    halves.

40. **The butterflies were drawn the size of a shrub and flew like birds.**

    The other half of the same complaint, and the same missing scale. A butterfly was drawn as a
    **whole tile** — `Rect::from_center_size(cell.center() + drift, Vec2::splat(scale))` — which at
    the pack's zooms is twelve to twenty-eight points across, as wide as a bush, and it was
    scattered over the level's *air*: one in a hundred and sixty empty tiles, anywhere from the
    grass line to ten tiles up, drifting an ellipse 0.34 of a tile across at 4.5 beats a second.
    Measured on the shipped levels that put three of the L4 frame's seven butterflies at rows four,
    eight and thirteen above the ground, wandering a third of a tile — which is a bird.

    Four numbers now, and each one is a claim about what the creature is. It is drawn at **0.45 of
    a tile**, bottom-anchored a tenth of a tile above the ground like a prop stands on it, so at the
    fitted zoom it is five or six points and at the play zoom twelve. Its drift is **0.16 of a tile**
    (and half of that vertically), small enough that the rectangle cannot leave its own cell and
    reach into the ground it is flying over. Its wingbeat is **7 Hz**, because a butterfly's beat is
    a blur at arm's length and a bird's is not, which is half of what tells the two apart. And it
    is drawn only in an empty tile with open air above **and solid ground below** — the third rule,
    and the one that changes what a level looks like: the pool is no longer the level's nine hundred
    tiles of air but the sixty-odd tiles of standing room along its ground, so the rate went from one
    in a hundred and sixty to one in fifteen to keep the count where it was. The pack holds twenty of
    them, three or four in a level, and L1's opening frame holds three — at the grass line, next to
    the three trunks.

    The test grew its own rule with the layer: `the_butterflies_drift_only_where_the_air_is` now
    requires ground under every butterfly it finds and a rectangle no wider than the size the view
    draws, on top of the old pair (empty tile, open above) and the wingbeat and drift checks. The
    drift check moved to two seconds of clock from a quarter of a beat, and that is part of the
    change rather than an accommodation: at a fifth of a tile over the ground, a quarter of a beat
    moves a butterfly by less than the pixel its rectangle is rounded to, so asking for movement
    between two wingbeats would be asking the drift to be *big* rather than asking it to be there.
    Frames: `out/scenery-butterflies.png` is the beach at 6x with the animals as specks just above
    the grass, and `out/scenery-after.html` puts that strip beside the size rule and the trees.

41. **The window opened on a level document, which is a cold start for anyone who has not
    used it before.** The editor is the right place for the app to *be* — it is the thing you
    came for — but the first frame should say what the thing is before it asks you to drag a
    tile out of a palette. `crates/app/src/splash.rs` is a title screen: the game's own sky,
    hills, trees and butterflies with a runner playing a short level behind the words, and any
    key or click leaves it for the editor exactly as the window opened before. It is the
    window's first frame unless the window was asked for a scripted one.

    **The world behind the title is the real renderer and the real simulation**, not a second
    animated backdrop: the same `view::draw_background`/`view::draw_sim` the play view calls,
    and the same `play::Session` the playtest drives, on the same fixed timestep, with a
    four-line script in place of a keyboard. A hand-drawn splash is a picture of the game that
    drifts away from the game, and every art or physics change would have to be made twice;
    this one cannot disagree with the level it is standing on. The level is built rather than
    loaded (`Level::starter` plus one gap and one step) because a splash has about two seconds
    of a person's attention and a shipped level is either too long to finish inside that or too
    plain to be worth watching.

    **The script had to discover that this game's jump is variable-height.** Its first version
    tapped jump at the edge of the pit and the runner fell in. A jump here is force-held for
    `min_jump_hold` (8 ticks) and *cut* to `jump_cut` (0.45) of its height if the button is
    released while still rising (findings 4, 5), so a tap is a 1.70-tile hop and not the
    4.00-tile arc the physics bible quotes for a full one. The script holds jump while
    `vy < 0.0` and lets go at the top of the arc, which is what a person does and is the only
    thing that gives the full arc — and letting go exactly there is also what stops the landing
    from being a fresh press, since a button still held when the feet touch down is a second
    jump (finding 4). That timing window is the constraint the shipped levels are built around,
    and the splash's own level is inside it rather than at its edge.

    **And the frame is framed, not fitted.** `Camera::fit` centres a level in the window and
    leaves two equal bands of sky; the words need them unequal — sky above and below the title,
    and a band of its own under the ground for "press any key" — so the screen draws from the
    fit *raised* by 7.5% of the window height. The one thing that could then drift from the
    framing is the camera the pull-back settles on, so the screen and its test both read the
    same function (`framed`) and the test additionally asserts that the invitation is drawn
    under the level's own bottom edge rather than over the lawn.

    Six tests hold it in place, and the first is the one that matters:
    `the_splash_runner_reaches_the_flag` drives the screen's own `advance` and requires the
    runner to *win*, because a title screen whose runner falls in the pit is a screen nobody
    will read twice. `a_key_or_a_click_opens_the_door_and_an_early_one_does_not` pins both
    halves of the doorway — an opening focus click or the key repeat of a held key must not eat
    the title, so events in the first 0.35 s are ignored — `the_title_arrives_before_the_
    invitation` pins the reveal to finishing before the invitation arrives to be read,
    `the_episode_ends_and_starts_again` pins the loop, `the_camera_pulls_back_to_the_whole_
    level` pins the framing, and `the_splash_draws_the_world_and_the_words` counts the
    tessellated output by texture — atlas quads for the world, glyph quads *on the title's own
    line* for the words — and requires the world to differ between two clocks, the title to be
    absent at the first frame and complete at the second.

    It is off for scripted frames by default (a splash in front of a script is a script waiting
    for a keypress that never comes), and `--splash` asks for it anyway: with `--screenshot`
    the screen is captured *finished*, because a still cannot show a one-and-a-half-second
    reveal and would otherwise photograph an empty sky. `--no-splash` is for the person who
    wants the editor on the very first frame. Frame: `out/splash.png`.

42. **The demo run was a demonstration of the game; the run's own champion is a demonstration
    of *theirs*.** The title screen (finding 41) came up playing a level built for it, which
    answers "what is this?" and not "what has it done for me?" — so when `champions/` holds a
    champion for the level the window is opening, that champion plays instead, on that level,
    and the credit line under the words names it: `the run's champion · generation 0 · fitness
    10908`. The demo run stays exactly where it was, as the fallback, which turns out to be its
    usual job rather than its rare one: of the eight champions in this repository's own output
    directory, four win and four die or time out, and all four of the shipped-pack ones that
    are not L1/L2 are losers.

    **The episode is re-decided, not replayed.** There is no recording of positions to play
    back — a champion file is a genome, a seed and the metadata that describes what it earned —
    so the picture is made the way the trainer made the number: `Sim::new(level, episode_seed)`,
    a network query on every tick the frame counter divides by the file's own `action_repeat`,
    and that action held in between. Doing it once per *frame* instead would have been the
    obvious thing and is the thing that does not work: a frame runs zero, one or two ticks
    depending on how long it took, so a frame-rate-dependent decision slides along the episode
    and the agent on the title screen becomes a different agent from the one the file
    describes. The seam that avoids it is `play::Session::advance_actions`, which hands the
    decision the *tick* instead of the frame, and `Session::advance` is now that with a
    constant decision, so the playtest and the champion share one implementation of the
    timestep. The test that pins it drives the screen to the end and asks it to agree with the
    file about the frame the episode finished on, the rightmost x it reached, the ground it
    covered and how it ended — a cadence off by one tick fails all four.

    **Three refusals, and one of them is the feature.** A champion from another build, or one
    trained on another level (the file records both the version trio and the level's content
    hash, and `evo::champion::replay` refuses either), and a best-ever agent that never
    reached the flag: a title screen looping a death is worse than the demo run it replaced,
    which is why the four losing champions above get the demo. Every refusal is silent — the
    file is a build artifact in a directory the window was *told* to look in, and a splash
    that refused to open over a stale one would be a worse bug than the one it reports.

    Two smaller things came out of it. **A champion's level is the window's own**, sixty tiles
    of L1 against the demo's thirty-four, so a champion is *followed* rather than fitted — at
    the window's follow zoom, the zoom the replay view follows its champion at, with the floor
    pinned at 78% of the window and only the x following the agent, because a camera that
    follows y as well bobs with every jump. Fitted, the same level is a stripe of lawn with the
    agent as a speck on it, which is a title screen about a speck. **And a champion is looked
    for under both of the names it can have**: `train-cli train` and `train-cli watch` write
    `<level file stem>.json`, the window's own Save champion writes `<slugged level name>.json`
    (`lab::stem`). For every shipped level those are the same string, which is how the
    divergence has gone unnoticed; for a level whose file name and title differ they are not,
    and a lookup that knew only one of them would be blind to half the champions on disk. Seven
    tests hold the lot: the episode agreement above, the four refusals, both names, the follow
    framing and the placement of the invitation — which had to stop being a fraction of the
    window once the world under it could be somebody's sixty-tile level. Frames:
    `out/splash-champion.png` is L1's champion playing L1, `out/splash.png` is L5's demo run.

## The M3 problem

The plan's M3 gate is "watch evolution for 10+ minutes and decide whether it is
fun". The benchmark above makes that gate harder to reach than the plan assumed:
a champion on L3 arrives in under a third of a second, so there is no 10-minute
arc to watch unless something changes. The levers, in the order worth trying:

1. **Pace generations, don't speed them up.** — *done, see finding 10.* Watch mode
   decouples training speed from viewing speed: training still runs at ~70
   generations per second, and the animation paces what the eye sees. The
   consequence is
   that a 10-minute session covers a few hundred generations on L1/L2 (solved at
   generation 0) and a few dozen on L3 (solved around generation 10–25), so the
   interesting part of an easy level is over in the first minute.
2. **Harder levels.** — *done, see [the difficulty ladder](#difficulty-the-generation-the-population-first-wins).*
   The pack now runs from "a random population wins" (L1, L2), through 16 generations
   (L4), to 68 — and sometimes never (L5). Getting there needed the measurements in
   findings 15–18, and the useful form of the goal turned out to be a *discoverable*
   ladder: every atom has to be learnable on its own before the next one arrives,
   which is exactly what the trap in finding 15 forbids. What did **not** work:
   length (a 124-tile marathon of hard atoms was never solved), walls (bunny-hopping
   clears them from generation 0), repeated obstacle types (the population
   generalises "jump when the gap is in front" within three generations), or
   walkers (they add texture and almost no difficulty).
3. **Diversity before fitness.** With pop 200 and no speciation the population can
   collapse onto "hop right" within a few generations — which is exactly the failure
   the champion-only view would hide. Best-4 + random-4 (Tier 1.5) makes it visible,
   and the death strip turns "where is it stuck" into a picture rather than a
   guess.
4. **Say when it has stopped moving.** — *done, see finding 19.* A stall line that
   reports how long since the population last found anything better, and which tile
   column its agents keep dying on. It is the difference between watching a level
   that is being learned and watching one that is not, and on the shipped pack it
   fires for real: 47 frames of a watch run on L5 seed 1, from generation 29, then
   gone once the flag falls at 76.

What is *not* answered by any of that is the gate itself. Tier 1 is complete and
exercised, and the instrument is now calibrated — a pack whose hard levels are 3.6
and 4.5 minutes at the default speed and 29 and 36 minutes at 1× — but "is watching
this fun for 10 minutes" is still a human judgement, and the honest status is: the
instrument exists and is pointed at a level hard enough to need it, and the
measurement has not been taken.
5. **Let the viewer turn the knob.** — *done, see finding 20.* `m`/`M` move the
   mutation rate along the plan's ladder (0.03–0.10) and the header shows the live
   value, so a stall is something the operator can act on rather than only read. The
   measurement that came with it is the useful part: the rate *decides* outcomes
   when chosen at launch, and switching it *at* a stall did not rescue one. The next
   lever is therefore not a slider but a different landscape — resume from the stuck
   point on an easier level.
6. **Show how the route changed, not just how the number did.** — *done, see finding
   21.* Tier 2.7's timeline scrubber: watch mode records the champion's route — one
   position every 4 frames — for each generation that improves the best, and
   `←`/`→` scrub back through them, drawing the recorded route over the level with
   `:` and ending it at the flag or at the grave. It is the one view that answers
   "what changed when it stopped being stuck", and on the shipped pack its span *is*
   the stall: seed 5's record runs `gen 0..38`, exactly the generation finding 19
   measured as the last improvement.

7. **Move the stuck population somewhere it can learn.** — *done, see finding 22.*
   `--curriculum <easier level>` makes the first stall checkpoint the population, train
   it there for 40 generations and hand it back, and `watch --resume` restarts from that
   checkpoint. On the seed that never solved L5 in any number of generations, this
   solves it (generation 163 via L3, 418 via L4). This closes the plan's stall chain —
   detector, death-spot highlight, resume, honest charts — with the measurement the
   plan asked for and the numbers the README can be held to.

8. **Hand the thing to a human, at a keyboard.** — *partly done, see finding 23.*
   `play` is M1's exit tool and the gate's final instrument, and it had never been
driven by a real terminal. Doing that found and fixed the reason a first playtest
looked broken (one frame per press), and it turned the exit criterion into two
measurements instead of an assumption: L1 and L2 are beatable by holding right and
jumping when something appears, L3 is not measured, and the whole check now runs in
CI (`crates/render/tests/playability.rs`) rather than in someone's memory.

## Deliberately deferred

NEAT/speciation, survival-analysis difficulty estimation, online champion
sharing, sound, mobile, powerups, achievements/tutorial, threaded WASM, and the
web build. See the plan's §12 for the gates.

No longer deferred: the pixel renderer, the editor and the watch mode. All three
existed as a terminal view and hand-written JSON through M1–M3, and are now a real
window (`crates/app`) — which means a non-developer can build a level, validate it,
play it and **evolve a solver on it without touching JSON** (the last third of M4's
exit criterion, in place as of finding 26). Still open inside it, in the order worth
doing: the curated starter pack M4 exits on (built with the editor, not by hand) and the
level list as a browsable shelf rather than an Open menu. The watch mode's last
terminal-only feature — the stall fallback of finding 22 and the resume that goes with
it — is in the window as of finding 28, so every M3 and M5 watch behaviour is reachable
from both front-ends.
