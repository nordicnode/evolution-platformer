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
| **M4** editor + content | **the shell and the evolution view work** — pixel renderer, generated art, drag-and-drop editor, in-editor playtest, and watch mode hosted in the window (chart, lanes, route scrubber, champion saving, findings 24–27). Not yet: the curated starter pack and a browsable level shelf |
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
(finding 22). Every generation that
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
cargo test --workspace                 # 283 tests
cargo build --release

# The game: draw a level, drag tiles out of a palette, play it yourself.
python3 tools/gen_tileset.py           # only if you change the art
cargo run --release -p app             # opens on levels/l1-flat-run.json
cargo run --release -p app -- --level levels/l6-marathon.json
#   drag a palette item onto the level to place it; click to paint
#   right-drag erases   middle-drag (or alt-drag) pans   wheel zooms
#   ctrl+Z / ctrl+Y undo   G grid   F fit   ctrl+S save   P play/pause-mode
#   in play: arrows or W/A/S/D move and jump, R restarts, P or Esc returns

# The same run inside the window: train on the level you are editing and watch it.
cargo run --release -p app -- --evolve --level levels/l2-first-gap.json
#   space pause   n one generation   -/+ speed   m/M mutation rate
#   left/right scrub the recorded champions   h back to live
#   s save the best-ever champion   P or Esc back to the editor
cargo run --release -p app -- --evolve --pop 60 --seed 5 --generations 8 \
    --screenshot out/evolve.png

# Render one frame to a PNG and exit (how the renderer is verified without a human).
cargo run --release -p app -- --screenshot out/frame.png

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

Tiles: `.` empty, `#` solid, `^` spike (lethal, not solid), `G` goal (normalized
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

283 tests, all green. `cargo test --workspace`.

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
carries an id out of the checkpoint.

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
   * **Terrain is picked by what surrounds it.** A solid tile is grass where the sky
     shows, dirt where it is buried, with left/right variants where a side is open,
     which is why eight terrain sprites cover a whole pack and a level needs no
     per-tile art choices.
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
   a PNG and exits, and the pipeline asserts on the pixels. L1 renders roughly 19 000
   terrain pixels across its 60 tiles — grass over dirt, with the flag at the far end
   — and L3 renders 2 100 more plus the steel of its spikes, so the diff between the
   two frames is exactly the terrain and the spikes and nothing else.

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
doing: the curated starter pack M4 exits on (built with the editor, not by hand), the
level list as a browsable shelf rather than an Open menu, and the curriculum step
(finding 22) in the window — the stall line and the death cluster are already there,
but a stalled run in the app can only be restarted, not handed an easier level.
