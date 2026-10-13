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
| **M3** watch mode + fun gate | **not started** — this is the decision point |
| M4 editor, M5 web/release | not started |

The deterministic core, the evolution loop and the reachability oracle are
implemented and tested. There is no graphical watch mode yet; `train-cli` prints
the learning curve and `train-cli play` lets a human play a level in the terminal.

## Measured numbers

The plan extrapolated "~1–3 s per generation for pop 200 on 6 cores" from a 2009
benchmark and asked for the real numbers early (M2). Real numbers, on the target
i5-9400 (6 cores), `--release`, pop 200:

| Level | Solved at generation | Wall clock |
|---|---|---|
| L1-flat-run | 0 (a random genome already wins) | 0.02 s |
| L2-first-gap | 0 (3 of 200 random genomes win) | 0.02 s |
| L3-gauntlet | 9–23 depending on seed | 0.1–0.3 s |

That is **~0.014 s per generation** and ~14 000 population-episodes per second —
roughly 100× faster than the plan's estimate. The plan said to profile if the
numbers were far off; the profile is that nothing needs optimising. The
consequence is a *product* problem, not a performance one, and it is the main
thing M3 has to answer: a full solve is over in a quarter of a second, so watch
mode cannot be "run training at 50× speed" — it needs to pace generations
deliberately, or the levels need to be much harder. See
[The M3 problem](#the-m3-problem).

## Workspace

```
crates/sim/        the deterministic core: tile grid, AABB physics, sensors, oracle
crates/evo/        fixed-topology genome, GA, fitness, champion files
crates/render/     read-only terminal views of sim state (ASCII frame, keyboard play)
crates/train-cli/  headless driver: train, reach, replay, play, render, sensors
levels/            hand-written *.json levels (L1-L3)
champions/         generated champion files (git-ignored)
```

Boundary rules from the plan (§4), enforced by review:

1. `crates/sim` has no `std::time`, no threads, no `HashMap` in hot paths, and no
   I/O beyond explicit JSON parsing. Its only dependencies are `serde`,
   `serde_json` and `rand_xoshiro`.
2. Everything the sim needs crosses the boundary as plain data: `&Level`,
   `AgentAction`, `seed: u64`.
3. `crates/render` only ever borrows `&Sim` and simulates nothing itself.

## Quick start

```bash
cargo test --workspace                 # 121 tests
cargo build --release

# Can this level be beaten at all? Milliseconds, no training.
./target/release/train-cli reach --all --levels-dir levels --verbose

# Evolve, print the fitness chart, save a champion.
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
(every tile column × 5 sub-tile offsets × 3 directions × jump-hold variants),
simulates each with the **real** physics stepper and collision query, and BFSes
from the spawn platform. On failure it names the first jump that cannot be made
and why ("gap too wide", "wall too tall").

## Testing

121 tests, all green. `cargo test --workspace`.

The load-bearing ones: `determinism_hash` (10 000 frames twice → identical hash,
plus a pinned golden), `parallel_determinism` (1 thread vs 6 → identical fitness
vectors), `golden_observation_hash_at_a_fixed_state` (the 90-float vector is
pinned), champion round-trip (save → load → re-simulate → identical trajectory
hash), the reachability cases (a 3-tile gap passes, a 5-tile gap reports "gap too
wide", a 5-tile wall reports "wall too tall"), physics corner cases (landing,
head-bump, wall-slide, stomp vs side-hit, spike death, pit death, goal trigger),
and the GA smoke test.

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
   impassable; five offsets fixed it. Getting the pre-take-off runway test
   backwards (checking beyond the ledge instead of behind it) made *every* pit
   uncrossable — the platform's outermost column is the one you clear a gap from,
   and it has no platform outside it.
7. **`level_hash` is `fnv1a64:<hex>`, not `sha256:<hex>`** as the plan sketches,
   to keep the workspace free of a hashing dependency for an integrity check.
8. **No `clap`.** `train-cli` hand-rolls its argument parsing to keep the
   dependency tree tiny; unknown options are rejected rather than ignored.
9. **No `rand` crate.** `rand_xoshiro` alone, with a hand-rolled Box–Muller, so
   the exact mutation stream is pinned by this repository's own code rather than
   by a dependency's choice of algorithm.

## The M3 problem

The plan's M3 gate is "watch evolution for 10+ minutes and decide whether it is
fun". The benchmark above makes that gate harder to reach than the plan assumed:
a champion on L3 arrives in a quarter of a second, so there is no 10-minute arc
to watch unless something changes. The levers, in the order worth trying:

1. **Pace generations, don't speed them up.** Watch mode should run a fixed
   number of *generations per second* (or per user keypress), decoupled entirely
   from how fast the sim can go. The speed control becomes "generations per
   second", not "physics steps per rendered frame".
2. **Harder levels.** The current three are trivially easy because continuous
   bunny-hopping clears pits, stairs and spikes. Difficulty should come from
   design (moving platforms, tighter spike spacing, walkers on ledges, timing
   windows) rather than from a bigger network.
3. **Diversity before fitness.** With pop 200 and no speciation, the population
   can collapse onto "hop right" within a few generations. Watching requires
   *variety*: best-4 + random-4 rendering (already planned as Tier 1.5) is the
   minimum, and rewarding behavioural diversity may be necessary.

## Deliberately deferred

NEAT/speciation, survival-analysis difficulty estimation, online champion
sharing, sound, mobile, powerups, achievements/tutorial, threaded WASM, the GUI
editor, and the web build. See the plan's §12 for the gates.
