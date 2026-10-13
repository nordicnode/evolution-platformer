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
| **M3** watch mode + fun gate | **Tier 1 done, fun gate open** — the owner's call |
| M4 editor, M5 web/release | not started |

The deterministic core, the evolution loop, the reachability oracle and the Tier 1
watch mode are implemented and tested. `train-cli watch` grows a population on
screen: fitness chart, best-4 + random-4 agents per generation, speed control,
pause, death markers and the genius-agent splash. The level pack behind it is now
six levels wide, and its two hardest are *hard* (findings 15–18). What remains of
M3 is the part no test can answer — whether watching it is fun — and that needs a
human at a keyboard. See [The M3 problem](#the-m3-problem).

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
crates/train-cli/  driver: train, watch, reach, replay, play, render, sensors
levels/            hand-written *.json levels (L1-L6, easy → brutal)
champions/         generated champion files (git-ignored)
```

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
cargo test --workspace                 # 189 tests
cargo build --release

# Can this level be beaten at all? Milliseconds, no training.
./target/release/train-cli reach --all --levels-dir levels --verbose

# Watch it learn: chart, best-4 + random-4 agents, deaths, genius splashes.
./target/release/train-cli watch --level levels/l5-tightrope.json
#   space pause   -/+ speed   n next generation   f follow/whole level
#   s save champion   q quit

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

189 tests, all green. `cargo test --workspace`.

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

Watch mode adds three that hold its honesty in place:
`every_showcase_lane_replays_the_episode_that_was_evaluated` (a lane animates the
episode that was scored, seed for seed, so the picture and the numbers agree),
`watching_does_not_change_what_is_evolved` (the showcase's RNG is separate, so
training is identical whether or not anyone is looking), and `pausing_freezes_the
physics_and_stops_burning_the_frame_budget` (a paused generation may not advance).
The watch layout itself is testable without a TTY because `--headless` renders
plain text frames; `cargo test` exercises the whole frame composition that way.

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

What is *not* answered by any of that is the gate itself. Tier 1 is complete and
exercised, and the instrument is now calibrated — a pack whose hard levels are 3.6
and 4.5 minutes at the default speed and 29 and 36 minutes at 1× — but "is watching
this fun for 10 minutes" is still a human judgement, and the honest status is: the
instrument exists and is pointed at a level hard enough to need it, and the
measurement has not been taken.

## Deliberately deferred

NEAT/speciation, survival-analysis difficulty estimation, online champion
sharing, sound, mobile, powerups, achievements/tutorial, threaded WASM, the GUI
editor, and the web build. See the plan's §12 for the gates.
