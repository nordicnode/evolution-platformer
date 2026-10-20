# Findings

This file holds the deep material that used to live in the README: the measured
numbers, the numbered findings and deviations from the plan, the verification
detail, and the work that is deliberately deferred. The README is now the
user-facing entry point — what the thing is and how to run it — and this file is
where the reasoning behind it lives.

The findings are **numbered and append-only**. Source comments cite them by number
(`crates/evo/src/fitness.rs` refers to findings 46–49, `crates/sim/src/level.rs` to
finding 47, and so on), so an existing number never gets reused or renumbered, and a
new finding goes on the end.

# Measured numbers

The plan extrapolated "~1–3 s per generation for pop 200 on 6 cores" from a 2009
benchmark and asked for the real numbers early (M2). Real numbers, on the target
i5-9400 (6 cores), `--release`, pop 200: **~0.014 s per generation**, ~14 000
population-episodes per second — roughly 100× faster than the plan's estimate. The
plan said to profile if the numbers were far off; the profile is that nothing needs
optimising. The consequence is a *product* problem, not a performance one: a solve
is over before the screen has finished drawing, so watch mode cannot be "run
training at 50× speed" and the levels have to carry the arc themselves. See
[The M3 problem](#the-m3-problem).

## Difficulty: the generation the population first wins

Every shipped level, twelve seeds each, pop 200, `--stop-on-win`, cap 420
generations (§9's "the population is itself the difficulty oracle"):

| Level | Tiles | Solved | Median | Fastest | Slowest | Won at generation 0 |
|---|---|---|---|---|---|---|
| L1-flat-run | 60 | 12/12 | 0 | 0 | 0 | 12 of 12 seeds |
| L2-first-gap | 60 | 12/12 | 1 | 0 | 5 | 5 of 12 seeds |
| L3-gauntlet | 60 | 12/12 | 20 | 6 | 51 | never |
| L4-rhythm | 67 | 12/12 | 9 | 2 | 16 | never |
| L5-tightrope | 70 | 12/12 | 14.5 | 5 | 53 | never |
| L6-marathon | 80 | 12/12 | 8.5 | 3 | 16 | never |
| L7-ascent | 60 | 12/12 | 12.5 | 1 | 90 | never |
| L7-wall-school | 60 | 12/12 | 2.5 | 0 | 8 | 1 of 12 seeds |
| L8-long-jump | 60 | 12/12 | 9.5 | 1 | 16 | never |
| L9-wait-and-climb | 60 | 12/12 | 67.5 | 30 | 205 | never |

Reproduce a row with:

```bash
for seed in $(seq 1 12); do
  ./target/release/train-cli train --level levels/l5-tightrope.json \
      --generations 420 --pop 200 --seed $seed --stop-on-win --quiet --out /tmp/out \
    | grep -o "solved at generation [0-9]*"
  echo
done
```

**What a row is worth re-running, and what it is not.** Reproducing a row here means reproducing its
**count**, not its generations: the same arm, level and binary solves **6 of 12** on seeds 1–12 and
**11 of 12** on seeds 13–24 (the detour-free chain arm of the search section: the same level, the same
pop, the same twelve seeds, the detour switched off), so the first-win generations of one batch are a
single draw from a wide
spread (the batch effect is measured in
[the search section](#what-the-search-can-be-and-what-the-stall-chain-is-worth)). The rows were each
measured once, at their own commit, and this pass re-ran exactly one of them — L9's. It reproduces the
count (**12 of 12**) and none of its twelve generations (median 77.5, fastest 28, slowest 394 against
the recorded 67.5, 30 and 205), which is consistent with the note below the table that its own numbers
were already re-measured once when fitness v8 re-rolled every episode: a count is a claim a reader can
check, and a generation is a claim about one draw.

L1 is the smoke test — a randomly initialised population wins it, so its interesting
minute is over before the first frame is drawn. L2 is a coin-flip: *five* of the
twelve seeds contain a genome that crosses its pit at generation 0, and the other
seven need one to five generations of evolution. It is no longer free, which is the
state the level should have been in all along (§8: it teaches the game's first
obstacle, so a policy that never lets go of the jump button should not clear it).
L3–L8 are the rest of the earned half, and every one of them is now solved on all
twelve seeds inside the 420-generation cap — the *shape* of the ladder is the interesting part of
this table, and the movement pass moved it: L3 (20) is the slowest median, and the
two levels that used to be the pack's hardest, L5 (14.5) and L6 (8.5), fell to the
middle of it. That movement is measured, not guessed, in finding 44, and the ability
levels L7 and L8 sit just below L3 because each of them is one decision learned —
but a decision the population cannot make at all until it finds the move.

L7-wall-school is the one row that is not part of the ladder's *length*: it is the
wall jump on its own, learns in a median of 2.5 generations, and exists because the
ascent asks for that move twice — see finding 45, which is also where the honest
limit of "just train it on an easier level" is measured.

**L9 is the row that is a different kind of level, and it is measured in a different
pass.** Every other row above is an *obstacle*: one thing to find, and the ladder is how
long the population takes to find it. L9 is a *chain* — a spike, a pit, a nineteen-tile
walker patrol, a two-tile hazard patch, a two-tile step, a three-tile wall climb, a
gap in the upper ledge, another spike and a two-tile climb — with the spawn at column 2
and the flag at column 56, so a genome has to make nine moves in a row rather than one, and the
ladder's own median (1-20) becomes 67.5. It is the only row here measured at the *current*
observation — **v8**, where the twelve rows above it are v6 readings, and the reason its numbers
moved rather than being re-quoted: v8 re-rolls every episode in every arm, so every first-win
generation in this row is a new draw and no old one carries over. Re-measured at pop 200, 400
generations, `--stop-on-win`, with the trainer's own defaults (a walker level is judged on three
patrol phases, `REVIEW.md` §4's trial rule), **all twelve seeds solve inside the cap**: 30, 33, 54,
54, 58, 67, 68, 70, 81, 82, 90 and 205 generations. What the trial rule is worth on the pack's only
chain level is separable on the same version — with `--no-auto-trials` (one patrol phase per
genome, which is what every number in this row was taken with before the rule existed) the same
twelve seeds give **10 of 12**, 33, 35, 54, 71, 75, 77, 98, 105, 163 and 356, with seeds 8 and 12
unsolved at 400 — so the rule is worth two seeds and about nine generations of median here, and the
level is still the slowest row in the table by more than a factor of three (67.5 against L3's 20). It
remains the point of the level
rather than a defect in it: the pack's other levels are asked a question a reflex can answer, and
this one is not. See `REVIEW.md` §4 for the same level run against the population's own memory,
which is the mechanism item 1 of that review says a level with a wait needs.

Finding 15 still explains why a stall is level information rather than a bug; what
changed is that the pack stalls less often now. The ability levels are the ones that
cannot be won by luck: the reachability oracle refuses L7-ascent and L7-wall-school
with wall jumps disabled, and L8 with sprint disabled, and every refusal is pinned by
`the_ability_levels_require_their_ability` in `crates/sim/tests/levels.rs`.

## The trap crossing: does a population learn the crumbling bridge?

The trap pass added a cracked tile (ground that leaves the level `CRUMBLE_TICKS` = 32 ticks after a
body stands on it) and asked the natural question about it — *can a population learn to cross a span
at speed rather than stopping on it?* `REVIEW.md` §2.13 is where the question was written down, with
the shape of the level that would answer it: a cracked span long enough that stopping is fatal, flat
runway either side so the fuse is the only new thing, and no second route.

`levels/scratch-trap-crossing.json` is that level: twelve tiles of cracked deck over a twenty-tile
pit whose floor is spikes, a two-tile hole at mid-span that has to be taken on the fly, three spikes
on the far ground, spawn at column 1 and flag at column 46. `train-cli reach --verbose` reads the
bridge as ground and reports it beatable (five platforms, five reached, start #1, goal #3), which is
the right answer to "beatable at all" and says nothing about the fuse — the oracle stands on a
cracked tile forever.

Twelve seeds, pop 200, 300 generations, `--stop-on-win`, the protocol every other number in this
section uses:

| seed | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| first win | **0** | **0** | 2 | **0** | **0** | 2 | 1 | **0** | 1 | 4 | **0** | 1 |
| winners in 200 | 1 | 1 | 2 | 1 | 1 | 2 | 2 | 1 | 3 | 1 | 1 | 2 |

**12 of 12 solved, mean first-win generation 0.9, median 0.5, worst 4, and six of the twelve are won
by the initial random population before any learning happens at all.** One genome in two hundred
crosses a twenty-tile spiked pit over a crumbling deck without having evolved; the rest find it in
one generation of ordinary breeding. The command, for reproduction:

```bash
./target/release/train-cli train --level levels/scratch-trap-crossing.json \
    --generations 300 --pop 200 --stop-on-win --seed 1
```

**So the answer to the question is yes, and the interesting part is why it is yes.** The fuse
punishes *hesitation*: a body that stops on the deck loses the tile under its feet 32 ticks later.
A randomly wired policy does not hesitate — it picks a direction and holds it, which is the whole
crossing — so the level asks for the one behaviour a random controller already exhibits, and the
population only has to find the genome that keeps moving long enough. The trap tiles are a *hazard*
like a spike, not a *puzzle* like a wall climb; they raise the price of a decision the population
already makes constantly ("keep going?", taken 4 ticks apart) and they do not create a decision it
does not make.

For comparison, the demo fixture that carries the same two tiles
(`levels/scratch-trap-tiles.json`) is not a test of anything: it has a pit *and* spring pads at the
bottom of it, so 11 genomes in 200 fall in and are thrown to the flag at generation 0. That is a
fine artifact for a screenshot and the reason the crossing fixture exists separately.

**What this does not establish.** It is not a difficulty claim about the *tiles*: the level has one
obstacle and the obstacle is a straight line. A course that makes the span hard has to make the body
*stop* or land precisely *on* the deck — a walker to wait out, a spike on the deck, a narrow landing,
a climb at the far lip — and none of those was measured here. Nor does it re-measure the ladder: a
rung whose worst seed is won in four generations belongs in the difficulty table only after the
table itself is re-measured against the build it would join, which is `REVIEW.md` §2.13's own
condition for adding a course to the pack. The fixture stays a fixture, and its number is here so
that the next pass can tell the difference between the tiles being easy and the *level* being easy.

## What the ladder costs to watch

Rendered frames per generation at pop 128, counted from `--headless` frames (a real
terminal draws them at 60 Hz, so 16 ms each, see finding 10):

| Level | `--speed 8` (default) | `--speed 50` | Median solve at 8× | Median solve at 1× |
|---|---|---|---|---|
| L3-gauntlet | 177 frames (2.8 s) | 30 (0.48 s) | 57 s | 7.6 min |
| L4-rhythm | 107 frames (1.7 s) | 18 (0.29 s) | 15 s | 2.1 min |
| L5-tightrope | 347 frames (5.6 s) | 57 (0.91 s) | 1.3 min | 10.7 min |
| L6-marathon | 151 frames (2.4 s) | 25 (0.40 s) | 21 s | 2.7 min |
| L7-ascent | 301 frames (4.8 s) | 49 (0.78 s) | 1.0 min | 8.0 min |

A generation lasts as long as its slowest showcase lane, so a level whose obstacles
kill early is *cheaper* per generation than a long one — L5 pays the most here, not
because it is the hardest but because its characters live longest in generation 1
(347 frames against L4's 107), and L6's longer floor is cheaper than L3's because
losing early is what most of its lanes do. These are *generation-1* numbers, drawn
before anything has been learned: once a population starts dying at the obstacle
blocking it, its episodes get shorter and its generations get cheaper, so read the
table as the cost of the opening minutes of an arc, not of all of it. The point for the M3 gate: the same speed
control still spans "over in seconds" to "ten minutes", so a level solving in a
few generations at 8× can be watched one generation at a time at 1×. Before this
pack, no speed setting helped, because L1/L2 were solved by the initial population
and L3 was over in under a minute at every speed.

## The clock: what rewarding a quick finish bought

The game had a frame cap and no clock: an episode that reached the flag in 6 s and one
that crawled there in 38 s of its 40 s budget scored the same, so the search was told
nothing about time and a viewer had nothing to race. This pass adds one — fitness **v8**
pays `SPEED_BONUS = 200` for the fraction of the level's own frame budget a **win** left
unspent (`SPEED_BONUS × (budget − frames) / budget`), the MAP-Elites archive keys its
cells on the finish band as well as on behaviour (48 niches → 240 cells), and the
window prints the run's quickest win, races it in the playtest, and reports the bands the
archive has filled. All four numbers a reader needs are the same number: `sim::frame_budget`
for the cap, the episode's own `frames` for the clock, `EpisodeStats::budget_frames` for the
budget the score is computed against, and `sim::format_time` to write any of them.

The reward is small against the score and large against what actually discriminates a
population. On the sixty-tile levels every shipped level is, the budget is 2 400 frames and
a full-window win collects all 200 — 1.8% of the 11 000 a win is worth. But a generation
of 200 genomes on L1 has ~170 winners, all within a few dozen points of each other on
progress, jumps and checkpoints, and the clock is worth up to 200 across that crowd. It is
the only term with enough range to re-rank a generation that has already solved the level,
which is exactly the state L1–L8 reach within a generation or two.

**Measured: the term is the only change, so `SPEED_BONUS = 0.0` is fitness v7.** Same build,
same seeds, pop 200, 60 generations, `--release`, on the target i5-9400. `clock` is the
run's quickest win, `best` the champion's own episode.

| Level | seed | clock (bonus 200) | clock (bonus 0) | champion frames (200) | champion frames (0) |
|---|---|---|---|---|---|
| L1-flat-run | 1 | 6.15 s | 6.15 s | 373 | 416 |
| L1-flat-run | 2 | 6.15 s | 6.15 s | 371 | 504 |
| L3-gauntlet | 1 | 6.47 s | 7.13 s | 402 | 437 |
| L3-gauntlet | 2 | 6.40 s | 7.80 s | 385 | 493 |
| L6-marathon | 1 | 8.93 s | 8.93 s | 624 | 655 |
| L6-marathon | 2 | 8.67 s | 8.58 s | 530 | 658 |

Read it as: **the champion is quicker or equal in five of six rows, and by up to 22%**
(L3 seed 2: 493 → 385 frames; L6 seed 2: 658 → 530). The run's *record* is where the
effect is honest rather than dramatic: on L1 both settings hit 6.15 s, because 369 frames
is the fastest a body can cross that level at all — the floor is the level's, not the
reward's — and on L6 seed 2 the record is 10 frames *worse* with the bonus, which is what
a different ranking of the same population looks like: the first genome to reach the flag
is a different genome once the ranking changes. So the claim this pass actually
establishes is the champion's, not the record's: **when a generation has already solved
the level, the clock decides which of its winners is kept**, and the kept one is quicker.

The archive's time axis is what makes that survive into later generations, and it is a
*diversity* change rather than a reward change — 240 cells instead of 48 means a niche now
holds a quick win and a slow win separately, so finding a faster route to the same
landing zone does not evict the slower one and a route found at one pace is not lost when
the pace changes. The band count is the run's speed coverage: 4/4 bands on L1 and L8 (wins
in all four quarters of the budget), 1–2/4 on L3 and L6 after sixty generations, which is
the reading that says a level is solved *once* rather than solved *well*.

The sizing was checked as early as it could be: 200 is about twelve tiles of progress, so
a win that skipped a checkpoint to save 100 frames would come out 7.7 points behind one
that collected it (8.3 points of clock against 16 for the zone), and 200 frames — a tenth
of the budget — still does not pay for one. That trade is now a test
(`a_quicker_win_cannot_be_bought_by_skipping_a_checkpoint`), because it is the one way this
term could buy a worse route instead of a faster one.

# Workspace

```
crates/sim/        the deterministic core: tile grid, AABB physics, sensors, oracle
crates/evo/        fixed-topology genome, GA, fitness, champion files
crates/render/     read-only views: ASCII frame/scene, keyboard play, pure text charts
crates/train-cli/  headless driver: train, watch, reach, replay, play, render, sensors
crates/app/        the game: pixel renderer, drag-and-drop level editor, playtest
levels/            hand-written *.json levels (L1-L8 plus the wall school, easy → brutal)
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

# Determinism

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

## The race: three things the playtest's clock got wrong

Three separate faults, found by reading the playtest against the panel it is supposed to
agree with rather than by anything failing.

**The ghost was not the agent whose time was quoted.** The dock races the run's *quickest
win* (`Packet::fastest`, a `FastWin` — four numbers and no network), while the ghost was
attached from `Packet::best`, the highest-**fitness** genome. Since fitness v8 those are
different agents the moment a run has two winners, and `best` need not be a winner at all:
the common case was a ghost that died in the first pit while the line above it announced a
time it had never run. The fix is not a better guess between the two records, it is to stop
having two: `Trainer::fastest_agent` now stores the **episode** the time was set on (the
genome, and the result carrying its seed) beside the time, written in the same branch of
`Trainer::advance` that sets `fastest`. So `fastest.frames == fastest_agent.result.frames`
is an invariant of one code path rather than a coincidence of two, and the playtest races
exactly the record it quotes. Pinned twice: `evo`'s
`a_run_reports_the_quickest_win_it_has_recorded` replays the stored genome and asserts it
wins on the recorded tick, and the app's `the_ghost_finishes_on_the_time_the_run_quotes`
drives the whole path — train until a win, attach, run, check the flag tick.

**A clock that had not started claimed a lead.** The race line was
`{mine} {where} — the AI's best is {theirs}, {gap}`, and the gap is a subtraction of two
frame counts. At 0.00 s every gap is a win, so the idle line read
`0.00s ready — the AI's best is 8.42s, 8.42s faster` — a claim to be eight seconds ahead for
standing still — and the lamp went gold on the same arithmetic. Both now read the clock's own
state first: no gap is quoted before the first input, and `beating` is `started && …`, because
a race that has not begun is not one you are winning.

**A turn is crisp on the ground and stays slow in the air.** "Clunky" here has a number
behind it: at the bare `run_accel` a reversal is an acceleration with a negative sign, so
crossing from `run_speed` through zero took **20 ticks (0.33 s)** and from sprint **30 ticks
(0.5 s)** — two thirds of a jump's airtime spent still travelling the way the player no longer
wants to go. `Physics::turn_accel_scale` (2.5) makes a **grounded** turn 8 ticks from run speed
and 12 from sprint.

**Grounded only, and that was measured rather than assumed.** The first attempt scaled the
turn everywhere and broke `walking_into_a_pit_is_still_fatal` — the test that exists to hold
"turning around costs more ground than the fall leaves", which is *why* every pit in the shipped
pack is lethal. With a crisp airborne turn a body that walks off a lip at running speed can
press back onto the pit wall before the world runs out, which turns each pit into a wall-climb
exercise and quietly changes the difficulty of the pack. The scale is therefore grounded-only,
and `a_turn_is_crisper_on_the_ground_than_in_the_air` pins both halves: **5 ticks** grounded
(the bible's `turn_ticks` ceiling) against **17 airborne** (the unscaled figure, unchanged),
with the pit test still passing as the constraint that decided it. This is a physics constant,
so the workspace moved to **0.9.0**.

## The replay: which record the window follows, and which clock it reads

Reported from a watched replay, and both halves were real: **the clock kept counting after the
agent had reached the goal**, and **the agent being followed was overtaken by ghosts that reached
the flag first** — so the thing on screen, racing a quoted time, was not the agent that set it.

**The star was the champion.** `Replay::with_limit` took the episode from `Packet::best` — the
highest-*fitness* genome — while the dock quotes `Packet::fastest` and the playtest's ghost is
built from `Packet::fastest_agent` (the fix above). Those are different agents as soon as a run
has two winners, and the ordering is not exotic: `SPEED_BONUS` is 200 — twelve tiles — against
`CHECKPOINT_BONUS` 16 and `ROUTE_HOP_BONUS` 32, so a winner whose route closes a dozen more hops
than a quicker one collects 384 points of route against at most a few dozen points of clock and
takes the champion's place on fitness alone. The replay was therefore the one view still on the
old record: a slower agent in the middle of a field of agents that had beaten its time, with the
strip quoting that time beside it.

The two records are both kept, because they answer different questions (`FastWin` vs `BestEver`);
what changed is which one a *stopwatch* view runs. The star now comes from
`Packet::fastest_agent` — the episode `fastest` was measured on, written beside it by the trainer —
with `packet.best` as the fallback for a run that has never won, where there is no clock to race
and the best-scoring attempt is the most a replay can show (`replay::StarBasis` carries which of
the two it is, and the strip, the panel and the details board all say it in the same words instead
of calling both "the best agent"). `--replay`'s help, the replay's `f` row and the playtest's
ghost button were carrying the same misnomer — the button said "champion" while racing the
quickest win — and were corrected with it.

**The clock was the field's.** The strip and the panel read `Replay::clock`, which counts the ticks
the *replay* has been open: it keeps running while the slower ghosts behind the star are still out
there (that is the point of running them together) and the scenery is animated on it. Nothing that
quotes an episode's time may use it. The number a viewer reads is now `Replay::star_elapsed` — the
star's own frame count, the same unit the recorded episode is kept in, which stops counting the
moment the episode ends — so a win in 5.2 s keeps reading 5.2 s at the flag with the ghosts still
running. The playtest had this right all along (`play::Session::frames`, wall clock only for
animation); the replay is the same rule.

**The champion file is deliberately unchanged.** `Save champion`, `train-cli train`'s output and
the splash screen still record and play the highest-*fitness* genome, because the file answers "which
genome is the best one this run found" and fitness is that question, clock and all; the champion is
what `train-cli replay` re-verifies and what the title screen shows off. So "the champion" and "the
agent a stopwatch view runs" are two different agents on a run with two winners, by design, and the
UI now says which is which rather than leaving a viewer to infer it.

**Pinned.** `the_star_is_the_runs_quickest_win_rather_than_its_best_score` builds the state a real
run reaches with two winners — the quickest episode on the record, the slowest agent as the
champion — and asserts the star is the record's agent, that the champion is one of the *ghosts*,
and that a packet with no champion at all is still watchable. `no_ghost_reaches_the_flag_before_the_star`
is the end-to-end version on a real run that really wins (a flag two tiles from the spawn, so a
generation of eight contains winners): the star takes the flag in **17 frames**, the two winning
ghosts in **20** and **24**, and the test refuses to pass on a fixture where no ghost won, which
would compare nothing. `the_clock_stops_where_the_star_stopped` runs ten more seconds of wall clock
with the field's clock still climbing and asserts the episode time and the strip have not moved.

## What the watcher's core is worth

A run in the window stutters *while it is working*, and the reason is structural rather than a
hot loop: `evaluate_suite` is a `par_iter`, so a generation is spread over every core by
rayon — including the core the frame being drawn is running on. The window is not slow, it is
queued behind its own trainer. The worker now evaluates on its own pool sized
`available_parallelism() - 1` (`lab::training_threads`), which leaves the frame one core.

**Measured on a 6-core machine**, `l6-marathon` at pop 200, four generations, twice:

| rayon threads | training |
|---|---|
| 6 (every core) | 0.052 / 0.054 s/gen |
| 5 (one reserved) | 0.058 / 0.058 s/gen |

and the same reservation measured from the *other* side — a 5 ms timer's overshoot while the
trainer saturates the machine, 200 samples, which is what a frame waiting to be drawn
experiences:

| | median | p95 | worst | over a 10 ms budget |
|---|---|---|---|---|
| no trainer | 0.09 ms | 0.93 ms | 4.55 ms | 0 |
| trainer on 6 cores | 0.33 ms | 1.74 ms | **16.45 ms** | 1/200 |
| trainer on 5 cores | 0.29 ms | **1.09 ms** | **2.22 ms** | 0 |

The medians barely move and the **tail** is where the difference is: 16.45 ms is one missed
frame at 60 Hz, and reserving a core takes the worst case to 2.22 ms — inside the frame budget
with room to spare. So the trade is about 9 % of training throughput for a worst-case wakeup
that is seven times better, which is the right way round for a view whose whole purpose is to
be watched while it trains.

The same pass removed a second full-population copy per generation: `Generator::next_packet`
cloned every genome to build `Packet::agents` out of a `Vec<Genome>` it owned and never read
again (~1.2 MB at pop 200, the project's own figure for one such copy). It now consumes that
vector. This one is not separately benchmarked — it is a pure subtraction of work on the
worker thread, visible in the code as `population.into_iter()`.

## What the search can be, and what the stall chain is worth

This pass asked the trainer four questions rather than the content one, and the first thing it did
was build the tool that makes asking them cheap: **`train-cli bench`** runs one of the recorded
*protocols* — a named `(level, pop, generations, seeds)` row — over every seed, records the arm and
every knob that was in force to `bench/<protocol>-<arm>.json`, and reads two arms back **paired**,
seed by seed, with the exact two-sided sign test over the seeds they disagree on. The file records
where each seed's population began as well as what the search was (`start: noise`, or the path of
the champion or checkpoint it was seeded from), because that is the one fact about a warm arm that
is not derivable from the arm itself.

**Every row below is detour-free, and that is a protocol boundary rather than a detail.** `bench`
drives `Trainer::step` one generation at a time, so the stall chain (§12 M5, findings 50–51) cannot
fire: these rows are what a population does on its own, with no practice excursion ever moving it.
`train`'s own default is the opposite — it *does* derive a detour when a run stalls — and the
difference between the two is the largest effect measured in this pass, so it gets its own section
at the end.

Protocol for every arm: the named level, pop 200, twelve seeds (1–12), `--stop-on-win`, and the
first-win generation as the measurement — `chain` is `levels/l9-wait-and-climb.json` at a
400-generation cap, `tower` is `levels/scratch-tower.json` and `hazard-floor` is
`levels/scratch-spike-tight.json`, both at 300.

```bash
./target/release/train-cli bench --protocol chain --arm default
./target/release/train-cli bench --protocol chain --arm hidden32 --hidden 32
./target/release/train-cli bench --compare bench/chain-default.json,bench/chain-hidden32.json
```

### The chain level: nine moves in a row

Seeds 1–12 unless the row says otherwise. The second `default` row is the same arm run over a
*second* batch of twelve seeds, and it is the most important line in this section.

| arm | solved | median | mean | paired against `default` |
|---|---|---|---|---|
| `default` (16 wide, mean ranking) | 6 / 12 | 130.5 | 131.2 | — |
| `default`, **seeds 13–24** | **11 / 12** | 97.0 | 127.5 | — |
| `--hidden 32` | 9 / 12 | 123.0 | 151.3 | 8 wins, 3 losses, p = 0.23 |
| `--hidden 8` | 8 / 12 | 109.5 | 126.1 | 7 wins, 3 losses, p = 0.34 |
| `--rank-pessimism 1.0` (worst trial) | 9 / 12 | 153.0 | 165.0 | 6 wins, 4 losses, p = 0.75 |
| `--from-champion`, an L8 champion that evolved for 3 generations | **10 / 12** | 44.0 | 70.0 | 9 wins, 2 losses, p = 0.065 |
| `--from-champion`, an L8 genome that evolved for none | **11 / 12** | 68.0 | 150.2 | 8 wins, 4 losses, p = 0.39 |
| `--from-champion`, that genome with its id rewritten to 0 | 11 / 12 | 115.0 | 148.2 | — |

### The plateau and the hazard floor

| level | arm | solved | median | mean | paired |
|---|---|---|---|---|---|
| `tower` (a plateau) | `default` | 7 / 12 | 142.0 | 166.4 | — |
| | `--hidden 32` | 6 / 12 | 199.5 | 184.7 | 6 wins by `default`, 4 by 32, p = 0.75 |
| `hazard-floor` (one safe tile between spikes) | `default` | 2 / 12 | 198.0 | 198.0 | — |
| | `--hidden 32` | 2 / 12 | **40.0** | 40.0 | 2–2, p = 1.0 |

**The first two rows are the finding, and they are about the measurement rather than the search.**
The same protocol, the same binary and the same level solve **6 of 12 seeds on seeds 1–12 and 11 of 12
on seeds 13–24** — a spread of five seeds between two batches of twelve, wider than every knob effect
in the table below it. So no row here is a claim about a knob until it clears that spread, and none
of them does:

1. **Capacity is unmeasured on the chain, and the direction on the plateau is against widening.** The
   32-wide net solved three more seeds than the default *in the batch it was run in* (9 against 6) and
   the 8-wide net two more — and the default's own second batch scored 11, more than either. The paired
   tests agree from the other side: p = 0.23 and p = 0.34. The tower adds one direction that survives
   its own batch: the plateau solved **7 of 12 with the shipped net against 6 of 12 with a 32-wide
   one** (median 142 against 199.5), which is not significant at twelve seeds but is a second level
   where the wider net does not help. The one thing the pair of readings supports is that this pack
   does not have a capacity problem: the flag ships, the 16-wide recipe stays, and the arm to believe
   is a 24-seed one.
2. **The pessimistic ranking neither helped nor hurt measurably** — nine seeds solved against six, but
   a *worse* median (153.0 against 130.5) and a paired 6–4, p = 0.75, which is the batch noise above.
   The knob ships at its default 0.0, and the negative result still earns its line: the cost finding 14
   describes (an elite re-rolled out of the run by a patrol phase it never saw) is real, and on this
   level it is not what holds the population back.
3. **Seeding is where this pass's first answer was wrong, and the control is what caught it.** Seeding
   the chain from L8's champion looked like the pass's one decisive result — 10 of 12 at a median of 44
   against 6 and 130.5 — and it is not:
   * **An unevolved genome seeds a run just as well.** A champion taken from generation 0 of L8 — a
     genome the population never selected on anything, whose whole career is one random draw and 197
     frames of stumbling — seeds a chain population that solves **11 of 12 at a median of 68**, as
     good as or better than the three-generation champion's arm.
   * **And the populations do not face the same episodes.** A genome id is part of every `episode_seed`
     (§5), so a population whose ids begin at 596 — the champion's own — faces different patrol phases
     and different level variation from one whose ids begin at 0, which is what a fresh population
     gets. That confound sits inside *every* seeded arm, and it can be taken out: the last row is the
     *same* unevolved genome in a champion file whose id has been rewritten to 0, so its shell lands on
     ids 0–199, exactly the id space `Trainer::new` draws. Same episodes, different population — and it
     solves 11 of 12 at a median of 115, no better than the default's own second batch.
   * What survives is a mechanism rather than a result. `--from-champion` is the right way to continue a
     run whose checkpoint was never written, and to fine-tune on a level that was edited since; it is
     **not** evidence that ability transfers between levels, and the claim this pass wanted to make
     needs a bigger sample and an id-matched control designed into the arm rather than bolted on after.
4. **The hazard floor's one interesting number still has no sample behind it.** Both arms solved two
   seeds, but the wide net's two solves arrive at generations **34 and 46** where the default's two
   arrive at **176 and 220** — a factor of four or five on a floor whose whole difficulty is a long
   grind across one-tile spikes. Two seeds is not a measurement, so this stays a hypothesis rather
   than becoming a finding.

**A cross-check worth having: `bench` and the hand loops agree exactly.** The detour-free chain arm
below (`for seed in $(seq 1 12)`, `train --curriculum-generations 0`) gives per-seed first wins
**227 / – / 179 / 82 / 218 / – / – / – / 28 / – / – / 53**, and the `chain` protocol's `default` arm
gives those same twelve numbers, seed for seed, and the same six solves. The tool is not a second
implementation that happens to agree: it drives the same `Trainer::step`, and the same
`seeded_trainer` a `train` run drives, which is why a warm arm of the protocol and a warm `train`
run begin their populations the same way.

### What the stall chain is worth: 24 of 24, against 17 and 9

`train`'s default is to derive a detour when a run stalls, and this pass could not find a table
measuring what that is worth. The design write-up (findings 50–51) documents the derived practice
level — the same level from the furthest stand the best genome has reached, with the flag moved to
the stand the route reaches next — and measures the *jump-timing seeds* inside a detour as flat,
which is a different question from "does the detour make a hard level fall". It does.

**The first reading of this was nearly an artifact, and a second batch rather than a bigger claim is
what settled it.** One batch of twelve seeds put the chain at 6 of 12 without the detour and 12 of 12
with it, and the tower at 7 of 12 against 12 of 12 — large enough to need a second sample before it
deserved a sentence, so the same protocol was run again on twelve seeds never used before (13–24).
The chain's detour-free arm reads **11 of 12** there and the tower's reads **2 of 12**: the same
binary, the same level and the same protocol swings by five seeds in *each* direction on which
twelve indices it draws. (The seeding arm's id-matched control is a third reading of that arm from a
third direction — an unevolved, clustered population on ids 0–199, the id space a fresh run draws
in, also solves 11 of 12, median 115.) So the detour-free numbers below are quoted per batch,
paired against the detour arm on the *same* seeds, and the pooled figure is a count of paired
outcomes rather than a solve rate.

Same protocol as above, run through `train` instead of `bench` (the stall chain is `train`'s
mechanism, so the tool that deliberately has none cannot measure it), two batches of twelve seeds.
A batch is `(level, seed k)` for k = 1…12, run off and on at the same pop and generation budget:

| level | batch | detour | solved | median | mean | first wins, in batch order |
|---|---|---|---|---|---|---|
| `l9-wait-and-climb` (chain) | 1–12 | off (`--curriculum-generations 0`) | 6 / 12 | 130.5 | 131.2 | 227, –, 179, 82, 218, –, –, –, 28, –, –, 53 |
| | | **on** (the default) | **12 / 12** | **77.5** | **101.7** | 46, 111, 124, 51, 41, 74, 113, 81, 28, 394, 104, 53 |
| | 13–24 | off | 11 / 12 | 97.0 | 127.5 | 87, 57, 126, 97, 71, 90, 301, 166, 61, –, 172, 175 |
| | | **on** | **12 / 12** | **54.5** | **94.8** | 49, 60, 77, 192, 42, 76, 89, 389, 46, 47, 36, 35 |
| | **both** | off / on | **17 / 24** / **24 / 24** | | | |
| `scratch-tower` (a plateau) | 1–12 | off | 7 / 12 | 142.0 | 166.4 | –, 281, –, 71, 82, –, –, 157, 142, 291, 141, – |
| | | **on** | **12 / 12** | **87.0** | **109.8** | 115, 135, 67, 69, 82, 256, 193, 76, 39, 92, 80, 114 |
| | 13–24 | off | 2 / 12 | 166.5 | 166.5 | –, –, –, –, –, 106, –, –, 227, –, –, – |
| | | **on** | **12 / 12** | **88.5** | **97.2** | 98, 83, 85, 102, 80, 42, 92, 135, 152, 68, 192, 37 |
| | **both** | off / on | **9 / 24** / **24 / 24** | | | |

Paired *within* a batch the detour loses nothing, and the per-batch p-values say how much each batch
can carry on its own. Chain, seeds 1–12: ten discordant pairs, all ten the detour's way (six seeds it
wins outright, four it solves sooner, seeds 9 and 12 tying at 28 and 53), `2 × 2⁻¹⁰` = **p = 0.002**.
Tower, seeds 1–12: eleven to none (**p = 0.0010**), and twelve to none on the second batch
(**p = 0.0005**). Chain, seeds 13–24, is the batch that keeps the headline honest: the detour-free
arm already solves eleven of those twelve, the detour takes the one it missed and is faster on eight
of the eleven they share, but three come *sooner without it* — seed 14 at 57 against 60, seed 16 at
97 against 192, seed 20 at 166 against 389 — so nine pairs fall one way against three the other,
p = 0.15, which is not a result at twelve seeds. Pooled over both levels and both batches the pairs
run **42 the detour's way against 3 the other way** (p ≈ 9 × 10⁻¹⁰), and the pooled counts are **17 of
24 without the detour against 24 of 24 with it** on the chain and **9 of 24 against 24 of 24** on the
tower.

And what the detour buys is not only "the level falls". Where the detour-free arm was already strong
— the chain on seeds 13–24, 11 of 12 — the detour still moves the median from **97.0 to 54.5**
generations, and on the tower from 142.0 to 87.0 in the first batch and from 166.5 to 88.5 in the
second. Across its 24 solves the detour arm's median first win is 67 generations, and its slowest
solve in 48 attempts is still 394.

The derived detour is not a nicety on top of the search; on this pack's two hardest levels it is
what turns a coin-flip into a certainty, and — where the level is already winnable — what turns a
slow win into a fast one. Across 96 runs over 48 (level, seed) pairs it never turns a solve into a
miss, and it is the default every other table in this file was taken under, which those tables do
not say about themselves. That is checkable rather than assumed: the difficulty table's L9 row
(pop 200, cap 420, seeds 1–12, `train`'s defaults) was re-run under this binary and agrees on its
**count** — 12 of 12 — while disagreeing on every generation figure recorded in it (**median 77.5,
fastest 28, slowest 394** against the row's 67.5, 30 and 205), and the twelve numbers it produced are
the detour arm's own, seed for seed. The row is therefore a detour-included reading, and its
generations are not this binary's: its own text records that it was re-measured once already, when
fitness v8 re-rolled every episode, and this pass did not re-derive it a second time — the tables
above are the current readings. What the same twelve seed ids score *without* the detour is 6 of 12
on one batch, 11 of 12 on another and 11 of 12 under the seeding arm's id-matched control — exactly
why a headline taken from a single batch of twelve would have been wrong.

**What it costs.** A detour is not free: it spends `--curriculum-generations` (default 40)
generations on the derived level, and the run's main level is frozen while it does. So the trade is
wall clock in exchange for a level that falls, and it is taken once per stall spell rather than once
per run (the arm closes until the run finds something better than it had when the step fired). On a
level that falls anyway — the eight levels whose medians are 0–20 generations — the derived detour
never fires, because the chart never reports a stall, which is why the ladder's own numbers are
unchanged by it.

A two-seed timing spot check on the tower (seed 1 and seed 3, pop 200, 300-generation cap, so the
detour-free arm burns its whole budget) found the detour arm finishing in **6.5 s and 5.1 s against
6.1 s and 5.8 s** for the arm that never solved — three detour steps of 40 generations each, and no
wall clock paid, because a run that solves stops searching and a derived level is not the main
level. Two seeds is a spot check rather than a cost table; the general statement that is supported
is that the detour trades *generations* (40 per stall spell, on a level the main population is not
being scored on) for a level that falls.

**Limitations, stated rather than buried.** Every arm here is twelve seeds and the batch effect is
five seeds wide, which is why both batches are printed and why only the paired reading is quoted:
the four within-batch p-values are 0.002, 0.15, 0.0010 and 0.0005, and the only figure spanning an
independent second batch is the pooled 42–3. That pooled count is a count of *paired* outcomes
rather than a solve rate, because neither level's detour-free rate is stable on its own — the
tower's goes 7 of 12 and then 2 of 12, so its 9 of 24 is a batch artifact rather than a property of
the level. What nothing here contradicts is the direction: 42 discordant pairs one way and 3 the
other, and **24 of 24** solves where the detour-free arm manages 17 and 9. Second, the
`--curriculum-generations 0` arm is the protocol *as it stood before the stall chain existed* rather
than a control written beside it; here that is the same thing, and it is worth saying which arm
carries the burden. Third, the `tower` baseline measured here (7 of 12 at a median of 142, then 2 of
12) is *not* the `10 / 12, mean 94` that `REVIEW.md` §4's archive-gate table quotes for this plateau;
that reading is older than fitness v8 (which re-scored every level whose goal is not due right), so
this pass measured its own baseline rather than reusing it. The warm-start question this
protocol's arms raised is answered above, in the seeding arm, and by a control rather than by a
bigger sample.

## The route as a demonstration: what an oracle's walk is worth to a search

The reachability oracle (findings 52–60) answers *can this level be beaten* by sweeping canonical
moves with the sim's own stepper and assembling the survivors into a route graph. This pass added
the two things that answer turns into action: a **replay** that drives that route through the real
episode loop, and a **clone** that fits a genome to the walk the replay produced. The first is a
verdict nobody has to take on trust; the second is a warm start.

### The replay is an instrument, and it indicted its own oracle three times

`evo::route_play` walks the graph's route as a *player*: it steers to the current hop's take-off
position, presses the oracle's own script (`sim::reach::trajectory_action`, `climb_action`), re-reads
the graph when it lands, and reports an outcome plus a trace of every attempt. Walking the shipped
pack with it found three defects in the oracle, all of which are now fixed and all of which are the
kind a graph cannot see about itself:

* **A canonical script is indexed in physics ticks, not in decision blocks.** The sweep holds a jump
  for *n ticks*; a controller that re-issued the script once per block held a wall climb's jump for
  eighty ticks instead of twenty, and the climb never left the ground (`l7-wall-school`,
  `l7-ascent`). The replay now runs scripts per tick and makes only the *route* decisions per block.
* **A take-off position inside solid ground is not a position.** The sweep chose sub-tile offsets
  along every column of a stand, including offsets that put the body's box inside the step ahead of
  it; the physics then ejected the body out of the tile and the oracle recorded the landing, handing
  the graph a "cheapest" move up `l2-first-gap`'s staircase that no body could make. Fixed in the
  oracle (`takeoff_fits`, `MOVE_SET_VERSION` 3), found by the walk.
* **A take-off is a position *and* a speed.** `l2-first-gap`'s staircase sent the body back where it
  started until the approach matched the plan's own `launch_vx`: the body that arrives carrying the
  walk it came in on is not the body the sweep validated.

And one defect in the *replay's own aim*, which is the reason the press rule has the shape it now
has. The graph's take-off is a point on a twentieth-of-a-tile grid and the body is a box on a
four-tick cadence, so a press lands within half a block of its aim rather than on it — on a hop whose
margin is a fraction of a tile, half a block is the difference between the far ledge and the pit.
`l8-long-jump` was pressing a sprint jump 0.18 tiles past its aim and dying in the pit. The fix is
not a finer aim: the replay now **asks the physics** before it presses — `sim::reach::simulate_jump_at`
from the body's actual position and speed — and presses only where that sweep lands on the stand the
graph promised. `l9-wait-and-climb` is what that bought: it was one of the levels the replay could
not walk, and the trace shows why it now can (`held off ... : the local sweep does not clear the hop
from here`, then the press at the next position that does), and its verdict is eight route hops of
eight.

### Verdicts, per level and per episode seed

Ten seeds each, `EvalConfig::default()`, the shipped pack. `W` won, `D` died, `S` stuck
(`crates/evo/tests/route_play.rs` pins this table exactly):

| level | patrols | seeds 1–10 | reading |
|---|---|---|---|
| `l1-flat-run` | – | `WWWWWWWWWW` | a walk: distance and nothing else |
| `l2-first-gap` | – | `WWWWWWWWWW` | a jump |
| `l4-rhythm` | – | `WWWWWWWWWW` | repeated jumps |
| `l5-tightrope` | – | `WWWWWWWWWW` | a pit crossed at a precise take-off |
| `l7-ascent` | – | `WWWWWWWWWW` | a wall climb |
| `l7-wall-school` | – | `WWWWWWWWWW` | the climb with nothing else in it |
| `l3-gauntlet` | 1 | `WWDWDWDDWD` | the patrol's phase decides |
| `l9-wait-and-climb` | 1 | `WWDWDWDDWD` | as above; all eight hops when it walks |
| `l6-marathon` | 1 | `DDDDDDDDDD` | **all five route hops closed, then killed on the last stretch** |
| `l8-long-jump` | 1 | `DDDDDDDDDD` | **open discrepancy, named below** |

Three things follow, and they are the reason the table is worth printing.

**A route over static terrain is walked, and deterministically.** The six patrol-free levels are ten
of ten seeds, which is what "the oracle cannot disagree with the sim about how far a move goes" is
supposed to look like. The level with a patrol is the level whose verdict moves, and it moves with
the episode seed because that is where the patrol phase comes from: **a patrol is the one thing a
plan over static terrain cannot express**, so the honest verdict on a patrolled level is a rate.

**Two failures are legible, and different from each other.** `l6-marathon` closes all five of its
route hops and dies on the walk to the flag — the route is *executed*, and what kills the body is a
patrol on the last stretch, which is a different sentence from "the crossing failed" and is why the
trace exists. This is the level-fairness question a level author actually asks.

**`l8-long-jump` is pinned as an open discrepancy rather than tuned away.** Its press cannot be made
executable: the local sweep says the move clears the hop from the body's position and speed, and the
live flight lands short of the promised stand. Same position, same speed, same action — so the gap is
the oracle's **clean spawn state** (standing at the surface, `vy = 0`, speed cap set from the launch
speed) against the state a live body carries into the press, and on a five-tile pit against a sprint
jump that difference *is* the landing. It is left in the table with its trace so the next pass starts
from the trace instead of from the surprise.

### The clone: a demonstration is not a policy

`evo::imitate` fits a genome to the replay's demonstration — the `(observation, action)` pair at
every decision block — by regression on the demonstrated action (the chosen action's `tanh` output
pulled to `+1`, every other to `−1`), full-batch gradient descent, exemplars weighted by inverse
action frequency and a memory shape trained free-running. It learns the demonstration: **95–100%
agreement**, against 0–10% for the untrained initialisation.

It also does not play the levels, and the first explanation written here — that the observation
*aliases* the press, so the demonstrated press is not a function of the demonstrated input — was
**tested and refuted**, which is worth recording as such. Group every decision of a demonstrated walk
by its own observation and ask whether two decisions that share a vector disagree about the action:
across seven walks (`l2`, `l3`, `l4`, `l5`, `l7-ascent`, `l9`, the `l2` mirror) there are repeated
states and **not one conflict** — neither on the full 98-wide vector nor on the projection with the
continuous goal and waypoint deltas dropped. Whatever the clone is failing to learn, it is not a
label the input contradicts.

What it is failing to learn is **time**, and the reason is the decision cadence rather than the
observation: `sim::reach` sweeps its canonical moves with the sim's own stepper and presses a take-off
*on the tick the body reaches it*, while a policy decides once per `action_repeat` — four ticks — and
holds its arm in between. The oracle's press therefore has no tick of the policy's to be made on.
Three measurements, and the third is the one that settles it:

* **At the shipped cadence, fidelity does not help.** On `l2-first-gap` the clone reproduces 41 of the
  walk's 46 blocks at the default 300-epoch fit and walks into the pit; raised to a 4000-epoch fit it
  reproduces **all 139 of its decisions** — every one — and walks into the same pit, identically on
  all eight evaluation seeds tried.
* **Coarsening the oracle does not help either.** Forcing the *replay itself* to press only on a
  decision boundary breaks the walk: `l2`, `l4` and `l9-wait-and-climb` die at their first gap in
  under 200 frames. The tick-precise press is what the walk needs, not an artefact of how it is
  recorded.
* **Change the grid and the claim flips.** With `action_repeat = 1` — a decision and a script tick are
  then the same tick — the same fit, on the same level, with the same evaluation seeds, **solves it on
  all eight, with identical frame counts**, because a clone that reproduces every decision *is* walking
  the route the oracle walked. `l4-rhythm` and `l5-tightrope` do the same; at the shipped cadence all
  three are 0 of 8 at every fit budget tried.

So "the clone walks the route and does not finish it" is neither a fitting failure nor an observation
failure: it is what a four-tick controller can do with a one-tick demonstration. The claim is pinned by
`a_clone_plays_the_level_only_at_the_demonstrations_own_cadence` in `crates/evo/tests/imitate.rs`, and
the honest reading of `--from-route` is "a head start at this cadence" rather than "a solver with a
fitting problem".

One thing the aliasing hunt did buy, because it is a different measurement: sensors v10 appends the
body's sub-tile phase (`phase_x, phase_y`) at indices 98 and 99. It was built for the refuted theory,
and then justified by the fit — **at a 4000-epoch fit the 100-wide vector misses 0 to 2 of a
demonstration's decisions where the 98-wide one misses 18 to 19** (551 decisions, `l2-first-gap`, the
oracle's own cadence), and at that cadence a fit that misses nothing is a solver on every seed while
the one that misses eighteen is a solver on none. The slots also close the one genuine hole in the
old vector: when the flag is further than `GOAL_VIEW_DISTANCE` (sixteen tiles), the goal vector is
*clamped*, and then the 98-wide vector contains no sub-tile position at all. It does not make the
shipped cadence work — nothing does — and a version bump re-scores every champion, so it is recorded
with its measurement in `sim::sensors`' own docs.

### What the clone *is* worth: a quarter of the population, and only a quarter

The clone is a walker even where it is not a solver, and a walker is a warm start. Two paired
measurements, both at pop 64 with the same master seed in every arm.

**Fitness, against a random population.** Mean over eight episode seeds, the clone's own fit at the
defaults against 32 random genomes on the same seeds (a random genome's *median* is exactly zero —
the score pays for progress and a random policy makes none):

| level | clone | random median | random best | clone's rank |
|---|---|---|---|---|
| `l2-first-gap` | 703.2 | 0.0 | 701.2 | 100th %ile |
| `l3-gauntlet` | 375.2 | 0.0 | 379.1 | 98th |
| `l4-rhythm` | 201.0 | 0.0 | 471.2 | 91st |
| `l5-tightrope` | 190.4 | 0.0 | 191.2 | 95th |
| `l7-ascent` | 108.7 | 0.0 | 114.0 | 91st |
| `l9-wait-and-climb` | 45.9 | 0.0 | 55.4 | 92nd |

So the clone starts every level near the top of what a random population reaches, which is the claim
`--from-route` makes. Note that `l3-gauntlet` and `l9-wait-and-climb` are levels the replay itself
cannot walk on those seeds: a **partial** walk is still a warm start, and it is worth what it
demonstrated and nothing past where the walk stopped — which the command prints.

**Generations to a first win, paired by seed.** Six seeds, fifty generations, first-win generation
summed over seeds (a miss counts as the cap), each arm a **full** population of 64 assembled the same
way (`resume`) so the only difference is how many slots the clone filled:

| warm of 64 | `l2-first-gap` | `l4-rhythm` | `l5-tightrope` | `l7-wall-school` |
|---|---|---|---|---|
| 0 (random) | 6/6, 59 | 5/6, 186 | 4/6, 195 | 6/6, 124 |
| 1 | 6/6, 56 | 6/6, 93 | 4/6, 207 | 6/6, 70 |
| 8 | 6/6, 37 | 5/6, 132 | 5/6, 156 | 6/6, 22 |
| **16 (the default)** | **6/6, 22** | **6/6, 71** | **6/6, 43** | 6/6, 75 |
| 64 (all of it) | 5/6, 142 | 4/6, 222 | 6/6, 87 | 5/6, 136 |

A quarter of the population is the arm that solves every level on every seed, two to four times
faster than random (`l2` 22 against 59, `l5` 43 against 195, `l4` 71 against 186). **Warming the
whole population is the worst arm** — worse than random on three of the four levels, and 20 of 24
solves against random's 21 of 24.

**Read that table with one caveat, added after it was caught reading a wash.** Every arm above was
built by `warm_population`, which draws the clone's slots *first* and fills the rest from the same
stream — so the arms differ in their random remainder as well as in how many slots the clone filled,
and a row-to-row difference is a difference of two populations rather than of one population's slots.
The *paired* measurement is the one pinned in `crates/evo/tests/imitate.rs`, where both arms hold the
same 48 random genomes under the same ids and the only thing that changes is the sixteen warm slots:
at pop 64, three seeds, the warm arm reaches a first win in **12 generations against random's 15** on
`l2-first-gap` and **24 against 130** on `l5-tightrope` (3 of 3 solves against 1 of 3). So the
direction the table claims is the direction the paired arms show, on the levels the table is about; the
survey's own totals are a wider sample of *arms*, not a tighter estimate of the mechanism. That is the shape of the thing and not a tuning detail: the clone's
basin is a high-fitness plateau with no win on it (it walks the route and cannot place the presses the
cadence puts between its decisions), and a population standing on one plateau has nothing to its left.
`--from-route` therefore
fills `pop / 4` slots and leaves the rest random, and the flag is `--warm`.

The first arm of the table is what `Trainer::from_champion` would have done with the clone — reheating
the whole population around one genome — so the measurement is also a caution about the existing
fine-tuning flag, on the level where it applies.

**Limitations, stated rather than buried.** Six seeds per arm: the paired direction is consistent
(warm-16 wins every level on every seed) but no single level's total is a tight estimate, and the
`l4` random arm's 5 of 6 makes that row's comparison a count rather than a rate. The clone is fitted
at the shipped defaults (`300` passes, `lr` 0.05, initial weights seeded from the fit's own seed), and
none of the arms above re-tuned them: the claim is about the *mechanism*, and the fit is one
hyperparameter triple inside it. The clone is not a solver on any level of this pack, and the module's
own docs say so at the top: what is measured here is a warm start.

## The version trio

`sensors::VERSION`, `evo::fitness::VERSION` and `sim::SIM_VERSION` are stamped
into every champion file along with `action_repeat` and the level hash. Loading
refuses a champion whose versions or level hash do not match, so an old champion
can never silently misbehave (§8).

The movement pass (findings 43–44) is the first change to exercise all three at once:
jumping became edge-triggered with a press buffer, three sensors and three actions
were added, and fitness gained a per-jump term — so sensors went to **v2**, fitness to
**v2** and the workspace to **0.2.0**, and every champion trained before it is refused.
Refused *for the right reason*, which is its own test: the new provenance field
(`jumps`) carries a serde default, so an older file parses and is then rejected by the
version check that explains why, rather than dying as malformed JSON and sending its
reader after a corrupted file that is not corrupted
(`a_champion_file_without_the_new_provenance_field_still_parses`).

The hop-length pass (findings 46–48) moved the workspace to **0.3.0** and fitness to
**v3**: the hold floor fell from two decision blocks to one, `brake` started biting in
the air, and the per-frame survival term was deleted. Sensors stayed at **v2** — the
observation was never the problem — which is the first time the trio has moved *apart*
rather than together, and it is the reason the field is a trio and not one number.

The checkpoint pass (finding 49) moved it again, to **0.4.0** and fitness **v4**: the sim
now counts the safe landing zones the body has stood on past a hazard or a hole, and the
score pays for them once each. No physics changed — but `EpisodeStats` is part of the state
`state_hash` pins and part of what a champion is replayed against, so a new field in it is
a version bump like any other. Sensors stayed at **v2** again.

The vertical pass — the objective itself — is the biggest of them, and it moved all three
at once for the first time since 0.2.0: **0.5.0**, sensors **v3**, fitness **v5**. What
changed is not a term but the direction every term is measured in (`progress_pixels` is
now distance closed on the flag rather than rightmost x), the frame budget
(`max(width, height) × 40`) and the goal observation (a two-axis vector rather than a
horizontal sign). All three are visible to an agent, and together they are the difference
between a tower being unwinnable and being learnable — measured on `levels/scratch-tower.json`
at two seeds in three, see the `evo::fitness` module docs.

The leftward pass moved it once more, to **0.6.0** with sensors **v4** and fitness unchanged
at **v5**: the action table gained the two leftward sprinting arms (`RetreatSprint`,
`RetreatSprintJump`, appended so no earlier index changed meaning) and the observation's
single directional gap probe became `gap_right` + `gap_left`. Fitness is untouched because
the *reward* was already direction-free — what was missing was the capability (there was no
leftward sprint for the population or the keyboard to ask for) and the perception (the probe
read `sign(vx)`, and therefore "right" for a body that had not moved yet). The sharpest
thing that changed is what the oracle now says: every shipped level's **mirror** is beatable
in principle, where the mirror of `l8-long-jump` was not before. See REVIEW.md findings 1.8
and 1.9.

The perception pass moved **sensors to v7** with the sim and the fitness untouched: v5 bounded the
nearest-enemy vector so a walker across the map stopped feeding a constant phantom threat, v6 widened
that bound from four tiles to eight after four was measured as *harder to learn* than the unbounded
lie (twelve seeds on `l3-gauntlet`: mean first win 41 generations at four tiles against 25 unbounded
and 18 at eight), and v7 appended `enemy_dir`, the nearest sighted walker's direction of travel.
That last one closes the observation's oldest gap about walkers: a body could see where one was and
not which way it was going, and the direction is not authored anywhere — it is drawn from the episode
RNG, which is what makes a walker level stochastic and a trial count meaningful. Nothing about the
physics, the reward or the frame budget moved with it, so the trio stays split, exactly as the field
is meant to allow. Every champion trained before v7 is refused at load (it asks for 96 inputs and
files carry the count they were built with); see REVIEW.md §2.9 for why the level format still has no
heading field to match the new slot.

The clock pass (REVIEW.md §4, "The clock") moved the trio the other way: fitness to **v8**, with the
workspace at **0.8.0** and sensors left at **v7**. The *sim* moved as well, which is why this one is a
pair and not a single bump: a new term needs a denominator, and the honest denominator is the level's
own frame cap, so `EpisodeStats` gained `budget_frames` — a field of a struct that is part of the state
`state_hash` pins and part of what a champion is replayed against, exactly as finding 49's checkpoint
counter was. Champion files from before v8 are refused by the version check, and `--replay` re-measures
a saved episode against the new arithmetic, which is the whole point of the trio: the alternative is a
file that replays to a *different score* than it advertises. See
[The clock: what rewarding a quick finish bought](#the-clock-what-rewarding-a-quick-finish-bought).

The spike's kill zone (REVIEW.md §2.12) moved the workspace to **0.10.0** with sensors and
fitness untouched: sensors are **v8** and fitness stays **v8**, because the observation and
the reward are unchanged — what moved is where the hazard *is*. (This entry said v7 when it
was written; the route waypoint vector's append — v8, `waypoint_dx`/`waypoint_dy` — landed in
the same commit, and `sim::sensors::VERSION` is 8 in that tree. The code is the authority on
which pass a version belongs to; the trap pass below is the first entry written against a
tree it could read.) The sprite has always drawn
its tips three of sixteen cell rows below the tile's top, deliberately, so that a spike
under a ceiling does not read as part of it; the kill zone tested the whole tile anyway. The
band that mismatch left above every spike was 0.1875 tiles tall and invisible, and it was
measured to be the difference between a level being solvable and not: on the hand-drawn
level that produced the report, one take-off position out of 81 reached the flag alive under
the tile rule, against fourteen of 81 under the drawn box, and 70% of the swept pairs died
without the body ever touching a drawn pixel. This is the first bump that moves **no**
observation and **no** reward, and the first whose evidence is a *level* rather than a
measurement of the pack — which is what a player-facing rule change should look like, and
why the trio is a trio.

The trap pass moved the workspace to **0.11.0** with sensors at **v9** and fitness left at
**v8**, and it is the first pass whose subject is the **world** rather than the body: a cracked
tile (`d`/`s`/`b`/`w`) is solid ground that leaves the level thirty-two ticks after a body stands
on it, and a spring pad (`v`) is a cell that throws a *falling* body upward at a second rung of
the jump ladder. Both are one glyph, both are behaviour rather than material, and between them
they are the first change to the sim that a *level file* can express and a champion has to be
re-measured against.

Three numbers carry the whole design, all of them derived rather than tuned. **`CRUMBLE_TICKS` =
32** — eight decision blocks, which is 3.2 tiles crossed at `run_speed` and 4.8 at sprint, so a
cracked span is a bridge crossed at a run and lost by hesitating, and a fuse the agent's own clock
divides evenly. **`spring_v0` = 0.35** gives `v0²/(2g)` = **4.90 tiles** of rise and **56 ticks** of
airtime against a held jump's 2.375 and 40: two jumps of height, 1.4 jumps of air, and a launch
zone exactly as tall as `max_fall` (0.75) so no fall can tunnel through the pad it should land on.
And the pad writes **only** `vy`, which is what "keeping its momentum" means concretely — the carry
is `vx × 56`, so **5.60 tiles** at run speed and **8.40** at sprint, and a level aims a pad at a
landing the way it aims a jump.

Sensors moved and the vector did **not** get any wider: `N_INPUTS` is still 98, no index changed
meaning, and what changed is what two cells of the patch *say* — a cracked tile reads `-0.25`
(between air and a spike, because it drops a body rather than killing it) and a pad reads `+0.5`,
the one value in the grid that says "worth aiming at". A patch that folded either into `ENC_SOLID`
or `ENC_EMPTY` would be telling the population something false about the world at exactly the moment
it decides whether to cross a bridge or to land on a pad, which is the precedent sand set in the
opposite direction: sand *is* a material and reads as soil, because physics draws no distinction.
Fitness is untouched because the reward is: the pad's throw is deliberately not charged as a jump
the agent made (`EpisodeStats::jumps` counts the button, not the bounce), so a level cannot make a
policy pay for being thrown.

The sim did move, and that is why this is not a sensors-only bump: a tile that removes itself is new
*state*. `Sim` carries the tiles that have left the level (`broken`) and the fuses that are burning
(`crumbles`), both folded into `state_hash`, and the long-runway golden hash was re-recorded for it
while the episode itself stayed bit-identical — that runway has neither tile, so the two readings are
empty from the first frame to the last, which is the honest shape of a hash change (`Sim::broken`'s
note).

What is verified: 563 tests green, including `a_spring_launches_a_falling_body_upward_and_keeps_its_momentum`
(pad height measured against `PHYSICS::spring_height()`, momentum asserted as "the launch frame moved
`vx` by no more than the body's own steering can explain"), a fuse counted frame by frame against
`CRUMBLE_TICKS`, and the rules' *negative* halves — a body rising through a pad, a body walking over
one, and a walker patrol on a cracked floor all do nothing. End to end, on the pass's own fixture
(`levels/scratch-trap-tiles.json`: a nine-tile cracked span over a pit with three pads at the bottom
of it), `reach --level` reports it beatable and the terminal renderer at `--action run` shows both
objects working in one run: the deck goes out tile by tile behind the runner, and the pads throw it
out of the pit and past the flag.

What is **not** measured, and should be read as open: no champion has been trained on a level with
either tile in it, and the pack's ladder has no rung that teaches them (REVIEW.md §2.13). The fuse
length is an argument about the decision cadence rather than a reading, and the two claims that would
settle it — that a population learns to cross a bridge at speed, and that the patch's two new values
are enough for it to *find* a pad — are a training run each. Until then the honest statement is that
both tiles are correct as physics, visible to an agent, and unproven as curriculum.

The enemies-and-objects pass moved the workspace to **0.12.1**, sensors to **v11**, the level format
to **v2**, and left fitness at **v8**. It is the largest single widening of the *world* the pack has
had: three enemies (a chaser, a flyer, a turret with projectiles) and two objects (a one-way platform
tile and a moving platform entity) all in one go, which is why every one of them was designed to fit
an existing rule rather than to add a mode. The chaser is the sim's existing `Walker` with a
`WalkerMode::Chase` and no new contact rule; a flyer reuses the stomp rule one row up; a turret's shot
is the pack's first hazard that moves and cannot be stomped, and the one-way shelf is the first tile
whose answer to `is_solid` and `is_opaque` differs. That last distinction is the whole design: the
player's x-resolution and ceiling test ask `is_opaque`, the landing probes, the walkers' floor probe,
the route graph and the sensors' patch ask `is_solid`, and one tile is the difference between the two.

Sensors moved to **v11** and the vector did **not** get wider: `N_INPUTS` stays 100 and no index
changed meaning. What changed is what the nearest-threat triple (81, 82, 95) *searches* over — flyers
and live shots as well as walkers, nearest wins, ties keeping the earlier kind so a level with none of
the new enemies observes exactly what it did before — and that a moving platform is drawn into the
patch as one-way ground, so an agent can plan a landing on a shelf it can see coming. v11 was the
number a previous pass had provisionally taken for a held-arm slot that was measured and reverted
(finding 62); the slot is not in the tree, so the version was free, and this pass is the one that uses
it.

The level format went to **v2** because a v1 build meeting a `=` row would call the level corrupt and
one meeting a `turret` would refuse the entity, so the file has to say which alphabet it is written
in. The *reader* accepts **1..=2** rather than only 2, and that is deliberate: v2 only added glyphs,
entities and an optional set of entity fields whose absence means "none", so every v1 file means
exactly what it meant and the shipped pack did not need migrating. The writer always emits 2. A future
v3 has no such promise to make, and `level_format_version 3 must be refused` is the test that keeps
the distinction honest.

The sim moved for the same reason the trap pass moved it: new state. `Sim` carries flyers, turrets,
shots and riders, and `state_hash` folds all of them in — including a *count* of each kind, even when
it is zero, so a level that has gained a flyer cannot hash like one that never had one. The long-runway
golden hash was re-recorded for it while the episode stayed bit-identical, which is the same honest
shape as the last two hash bumps.

**The pass needed one more version before anything was trained on it: 0.12.0 → 0.12.1.** The first
moving platform was janky in four separate ways, all of them found by playing it rather than by a test,
which is the sentence worth recording:

* **Carry order.** The rider was carried *after* its own tick, so gravity pulled it down from the
  shelf's old position and the landing snap pushed it back up — once per frame, forever. It is carried
  first now: the body begins its tick on the deck as it is.
* **The landing test was a position, not a crossing.** "The feet are above the deck" is true three
  tiles up, so a falling body was teleported onto a shelf it never crossed. A landing now needs the
  feet above the deck last frame *and* at or below it now, which is the same surface test the tiles use.
* **The shelf passed through terrain.** Nothing clamped the travel, so a platform authored through a
  wall slid through it. `Mover.limit` is the clear part of the path, computed once per episode from
  the same `is_opaque` test the body's collision uses, so a shelf turns at the first wall or ceiling
  instead — and its rider stops with it.
* **The view snapped it to whole tiles.** `draw_mover` drew in the *cell* the shelf was over, so a
  platform moving two hundredths of a tile per tick held still for fifty frames and then jumped a
  cell. It is drawn at its own continuous position now, with the sprite's top edge on the deck.

Two smaller things came with it: the flyer's beat is centred on the cell (measuring from the box's
left edge put it half a tile off the click and quietly shortened every span) and defaults to eight
tiles rather than four, and the turret's palette icon now follows its firing direction — the canvas
already mirrored a left-facing turret, so the button was the one place in the window promising the
other one.

**That was not the end of it: 0.12.1 → 0.13.0 is the same four things again, and the lesson is that
"fixed the bug" and "made the mechanic feel right" are two different passes.** Playing the 0.12.1
build produced the same three complaints word for word — a janky platform, a flyer with a tiny
flight path, a turret icon on the wrong way, plus a fourth (the projectiles) — and every one of them
turned out to be a *second* defect that the first fix had hidden:

* **The shelf was drawn continuously but not rigidly.** The 0.12.1 fix removed the tile snap and then
  cut each column where the *level's* grid fell, so at a fractional x the first piece was a fraction
  of a cell and the last was the sliver left over: the shelf's cut ends squeezed and stretched while
  it slid, and each tile boundary it crossed swapped which piece was which. It is `w` whole cells at
  `rect.x + i` now — one layout function, `mover_pieces`, with a test that every piece is exactly one
  cell wide and exactly one cell from the next at a fractional x. Continuous position was only half
  of "rigid".
* **The shelf's speed was chosen by feel and could not be drawn smoothly.** `mover_speed` was 0.02
  tiles a tick — half a screen pixel per tick at the usual zoom, which means the shelf holds still
  for two frames and then jumps one pixel, and the rider (snapped independently) slips a pixel
  against the deck while it does. The constant is **1/16 of a tile**, one 16-pixel art pixel a tick:
  the renderer's scale is a whole number of points per art tile, so every integer zoom moves the
  shelf exactly `scale` pixels a tick and the shelf and its rider step *together*. A derived speed
  rather than a tuned one, pinned by `a_platform_moves_one_art_pixel_a_tick`.
* **The landing test still measured the feet after the shelf had moved them.** `prev_bottom` was
  sampled after the carry, so on a rising lift the shelf's own motion was already counted as the
  body's — the crossing test passed for the wrong reason, and the ride was re-derived from scratch
  every frame instead of being kept. The feet are recorded *before* the carry now, and the ride is
  sticky while the body is still over the deck (`Sim::riding` is the state; a crossing is only how
  it starts). A lift test pins it: four hundred ticks of rise and fall, the feet never leave the
  deck.
* **The flyer's span was still short by the box's width.** 0.12.1 recentred the beat, which fixed the
  half-tile offset, but the ends still named the box's *edges*: bounds at `centre ± span/2` move a
  box of width `w` exactly `span - w` tiles. On a four-tile beat that is a fifth of the flight path
  — measurably "very small". `FlyerSpec::at` now adds the width back to both ends, so `span: 12`
  moves the flyer twelve tiles (`a_flyer_travels_the_span_it_was_given`), and the default rose from
  eight to twelve because eight was a band you could stand beside. `FlyerSpec::span()` is the inverse
  — writing the raw distance between the ends back to a file made the beat grow 0.7 tiles per save.
* **The turret's icon had two sources.** The palette read a second atlas cell (`turret_l`), the
  canvas mirrored `turret_r`; two pictures that are identical today and free to drift. There is now
  one function, `view::turret_picture(dir)`, and the palette and the drop ghost paint through
  `view::facing` with its `flip` half. The direction switch also had a real interaction bug: every
  segmented control in the palette shared a widget id (`ui.id().with(("segment", i))`), so the shape
  switch and the turret's direction switch fought over one hover and one click.
* **The projectile was slower than a walk and died in mid-air.** `shot_speed` was 0.08 against
  `run_speed` 0.10 and `sprint_speed` 0.15 while its own doc said "faster than a run, so a shot cannot
  be outrun" — a promise the physics did not keep, and a bullet a player can stroll away from. It is
  **0.18** now (above every gear the body has, below the terminal fall so the eye can track it) and
  `shot_range` is 18 rather than 12, with a **wall** as the stop and the range as the backstop for a
  row with none — so the demo's turret fires across a ledge and a five-tile pit and dies over the pit
  instead of vanishing in the middle of the air twelve tiles out. And the *picture* was wrong for it:
  a 16-pixel cell with a tail drawn into it, squeezed into a 0.3-tile `actor` box and stood on the
  box's floor, is a two-pixel dot floating above the row it kills in. It is drawn from the art's own
  geometry now — the hot core anchored on the hitbox, the tail trailing behind it, mirrored by `vx` —
  with a muzzle flash read off the turret's own cooldown so the *firing* is visible before the bullet
  arrives.

What is measured for 0.13.0: the whole workspace suite (635 tests, 224 of them the app's); the levels
still load and L10 still reaches (`platforms 4, reachable 4`); the flyer's travel is 12.0 for
`span: 12` against 7.99 for the same file under 0.12.1; the mover's 4-tile slide takes 64 ticks; a
shot crosses a 14-tile corridor and stops at the wall; and the faster bullet is still survivable —
`l10-the-hunt` at **pop 40, 60 generations, seed 1, `--stop-on-win`** won at **generation 12**, in
**9.67 s / 580 frames** (`best_x 56.3, jumps 7, checkpoints 3`, champion written). That is one arm on
one seed and it is slower than the 6.3-6.5 s of the 0.12 build, which is what a turret whose row now
covers the shelf crossing should cost — but it is a *win*, and it is the check the first pass never
ran on the numbers it shipped. What remains open: whether the twelve-tile beat and the 18-tile shot
are the right *difficulty* rather than merely survivable, and whether a 0.12 champion (none exists)
would have transferred.

# Agent interface

**Sensors (v11, 100 inputs).** A 9×9 tile patch around the body (solid `1.0`,
spike `-0.5`, enemy `-1.0`, empty `0.0`, cracked ground `-0.25`, spring pad `0.5`),
then nearest-enemy `dx`/`dy`, `gap_right`,
`grounded`, normalised `vx`/`vy`, `can_jump`, the goal vector, a bias input,
`gap_left`, `enemy_dir`, the route's next waypoint, and where the body is inside its own tile
(`phase_x`, `phase_y`) — the last one added by the v10 note below. The plan's sensor table omits spikes, which would
leave the most common hazard invisible; `-0.5` fills that gap and is documented in
`sensors.rs`.

v2 appended the three inputs the movement abilities need and changed what `vx`
*means*: `wall_dir` (the side the body is pressed into), `clinging`, and
`sprint_charge`. Every earlier index keeps its meaning and its position, so the diff
from v1 is additive. `vx` is now normalised by `sprint_speed` rather than
`run_speed`, because the old normaliser saturated at 1.0 for a running body — which
would have made a sprinting agent and a running one indistinguishable, the one thing
a speed sensor exists to tell apart. Running reads 0.67; sprinting reads 1.0.

v3 makes the goal a *vector* — `goal_dx` and `goal_dy`, each normalised by
`GOAL_VIEW_DISTANCE` — where v1 and v2 had a single `goal_dir` sign, the sign of the
horizontal difference. That sign could not express a flag directly overhead: from the
bottom of a shaft, a flag six rows up and a flag down a corridor both read as "not to
either side", which is why the first vertical level was unwinnable rather than merely
difficult. The vertical component is signed the same way the world is — y grows
downward, so a goal above reads negative — and nearest-*centre*, so the input is a
direction to steer by rather than a distance to arrive at.

v4 splits the single `gap_ahead` probe into `gap_right` and `gap_left`. The probe looked
in the direction of travel (`PlayerState::facing`, which read "right" for a body with no
velocity), so the observation's reading depended on the body's last decision — and the
first decision of an episode, where a direction has to be chosen, is exactly where that
hurts. The two slots now describe the ground beside each edge of the feet symmetrically,
and neither is "forward".

v5, v6 and v7 are the walker pass, in that order: the enemy vector gained a range (bounded
at four tiles, then widened to eight because four was measured *worse to learn from* than the
unbounded lie it replaced), and then it gained the one fact about a patrol that no amount of
training could recover — `enemy_dir`, which way the nearest sighted walker is walking. The
level format deliberately holds no such field (an authored heading would make every trial of a
walker level the same episode), so this is an observation change or it is nothing. See
REVIEW.md §2.9 and §4.

v8 is the route waypoint, appended at 96 and 97: the vector to the next landing stand along the
level's own move graph. It is the observation half of the same pass that gave the search its
route cache, and it exists because a memoryless policy at the foot of a left-going leg of a
zig-zag tower has nothing in its body state that says which way the leg goes.

v9 is the trap pass's re-reading and the cheapest kind of sensor change there is: the layout is
untouched — 98 inputs before and after, not one index re-meaninged — and what moved is what two
cells of the patch *say*, a cracked tile at `-0.25` and a pad at `+0.5`. Both are tiles whose
whole point is that they are not what they look like to a collider, so a patch that reported
them as the solid ground and the empty air they resemble would hide the two objects a trap level
is built out of.

v10 appends the body's sub-tile phase, `phase_x` and `phase_y`, at 98 and 99 — the one positional
quantity the vector did not carry, since the patch is keyed on `floor(centre)`, the gap probes on the
body's own edges, and the only continuous position signal left (the goal vector) is *clamped* beyond
sixteen tiles. It was built on a theory that did not survive its own test — the demonstration's press
is **not** observation-aliased, see "The clone: a demonstration is not a policy" — and it earns its
place on a different measurement: with the slots a 4000-epoch fit misses 0-2 of a demonstration's 551
decisions where the 98-wide vector misses 18-19, and at the oracle's own tick cadence a fit that misses
nothing solves `l2`, `l4` and `l5` on every seed while the one that misses eighteen solves none of
them. It is a fidelity fix, not a solver: at the shipped four-tick cadence the clone still cannot
place the press. It is a version bump, so every champion in the pack is re-scored rather than reused.

**Actions (11 macro-actions).** `run`, `run_jump`, `sprint`, `sprint_jump`,
`retreat`, `retreat_jump`, `brake`, `jump_in_place`, `walk`, `retreat_sprint`,
`retreat_sprint_jump`, chosen by argmax over 11 network outputs and held for
`action_repeat` (default 4) physics ticks. The table is *(direction × verb) with no
preferred direction*: sprint is a **gear**, not a direction, so `sprint` alone is still
a rightward run and there is no neutral sprint, but `retreat_sprint` asks for the same
gear to the left. The two leftward sprinting arms are appended rather than inserted,
because the index of an action is part of what a recorded genome means. `retreat_jump`
exists because a leftward jump is what makes a wall climbable from the keyboard as well
as from a genome, and the leftward sprint exists because without it the *mirror of*
`l8-long-jump` was unbeatable in the sim and in the reachability oracle alike.

**Network.** 100 → 16 (tanh) → 11 (tanh), 1803 genes. Fixed topology; NEAT is
deferred to M5 and gated on an observed plateau (§2.1).

**Fitness (v5).** `+1.0` per pixel of distance closed on the flag (from the *closest*
approach ever reached, so it can never go backwards), `-2.0` per jump taken, `+16.0`
per checkpoint zone stood on, `-20` on death, `+10 000` on touching the flag. Episodes
end after 600 frames without a new closest approach (anti-camping) or
`max(width, height) × 40` frames absolute. No proxy rewards: nothing scores jump
height, airtime or stomps.

v3 deleted a `+0.05 per frame alive` survival term, which paid for *not playing*: a
genome that parked in front of a hazard until the clock ran out out-scored one that
tried the crossing and died. v4 added the checkpoint term, which pays for arriving on
the far side of a hazard rather than for flying over one. v5 changed no term at all —
what changed is what "progress" *is*: rightmost x until 0.5.0, distance closed on the
flag since. On the shipped pack (every goal due right) it is the same number it always
was; on a tower it is the difference between solvable and not.

The jump term is the only negative one and it is aimed at the appearance of progress
rather than at progress: the cheapest jump that *does* something — clearing a one-tile
gap — earns 16.0, eight times its cost, so it cannot teach a population not to jump
when jumping is the answer; what it does is break a tie that was previously a dead heat
(`run` and `run_jump` reached the flag in the same number of frames for the same
progress, so selection had no reason to prefer either).

# Reachability oracle

`sim::reach` answers "is this level beatable at all?" in milliseconds, before any
training budget is spent. It extracts platforms, sweeps canonical take-offs
(every tile column × 20 sub-tile offsets at 0.05 granularity × 3 directions ×
jump-hold variants), simulates each with the **real** physics stepper and collision
query, and BFSes from the spawn platform. On failure it names the first jump that
cannot be made and why ("gap too wide", "wall too tall").

Its jump windows are a **lower bound** on what an agent can do (finding 18), so read
it as "beatable at all", never as "hard" or "easy": it is a safety check on a level
before a training budget is spent, not a difficulty meter. All nine shipped levels
pass it, including L3's spike patch, which the coarse five-offset sweep used to
report as impassable (finding 6).

It can also be asked to *withhold* an ability — `ReachOptions { wall_climb: false }`,
`{ sprint: false }`, or `{ spring: false }` — and that is how the pack's two ability levels
are pinned. The matrix is measured in `crates/sim/tests/levels.rs`: L1–L6 stay reachable
with either of the first two disabled (their geometry was already crossable), L7-ascent
and L7-wall-school become unreachable without wall jumps, and L8 becomes unreachable
without sprint. Spring pads are the third ability and the last to be modelled at all:
until finding 60 the oracle stepped a body one layer below the pad's own rule and called
a level whose only crossing was a pad unbeatable, and `{ spring: false }` is what pins that
shape now (`crates/sim/src/reach.rs`, `a_pad_at_the_lip_is_what_crosses_a_gap_no_jump_can`).
"Required by level N"
is therefore a property of the *level*, not of the agent that happens to solve it.

# Testing

563 tests, all green. `cargo test --workspace`.

The load-bearing ones: `determinism_hash` (10 000 frames twice → identical hash,
plus a pinned golden), `parallel_determinism` (1 thread vs 6 → identical fitness
vectors), `golden_observation_hash_at_a_fixed_state` (the 93-float vector is
pinned), champion round-trip (save → load → re-simulate → identical trajectory
hash), the reachability cases (a 3-tile gap passes, a 5-tile gap reports "gap too
wide", a 5-tile wall reports "wall too tall"), physics corner cases (landing,
head-bump, wall-slide, wall jump, a held button jumping once, a wall climb past a
single jump, sprint-jump reach, stomp vs side-hit, spike death, pit death, goal
trigger), the jump ladder (`every_extra_decision_block_of_hold_buys_height`: every block
of hold must buy height, one rung at a time), the air brake
(`braking_in_the_air_is_what_makes_a_short_hop_expressible`: no coasted hop fits a
one-tile pad at any length of hold, a braked one does), the checkpoint rule
(`a_landing_zone_is_a_checkpoint_only_when_something_had_to_be_crossed`, which pins the
spike, pit and step cases off the level geometry, and
`a_checkpoint_is_credited_on_arrival_and_not_for_flying_over_the_zone`, which pins the
*standing* requirement the term is defined by), and the GA smoke test.

Content gets its own guard rails, because level JSON is content and nothing else
checks it: every shipped level must load with the width, height and walker count the
pack table declares, be reachable in principle *with no caveats*, terminate inside
its frame cap, and keep its entities inside the grid and its walkers standing on
something (finding 17's off-grid walker is the mistake that test exists for). The
pack's difficulty is pinned by `crates/evo/tests/difficulty.rs`: a random
population must win L1 at generation 0, must win none of L2–L8 on any of four
seeds, L2 must be learned inside 20 generations at pop 64 (it is a coin-flip at pop
200, so the claim is "learned quickly when it is not free"), and L4 must still fall to
evolution inside 60 generations at pop 64.

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
and a release ends it at once, that the eleven actions are chosen in the plan's priority
order (jump beats direction, so a held direction while jumping is a real forward
jump; right beats left, so releasing right while left is held is a leftward jump and
not a neutral one; sprint refines a run *in either direction* without outranking the
brake, so shift-plus-arrow is that way's fast move in both the window and the
terminal), and that a
shifted key raises the sprint latch — because a terminal has no key for a modifier,
so the only evidence of shift is that it arrived attached to another key. Then quit and
restart fire once rather than every frame, a five-presses-a-
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
it, the flag can be erased and re-placed anywhere (a level has one, so the goal brush
moves it), a refused save leaves no file behind, and the beatable/not-beatable verdict
is the oracle's answer following a spike curtain drawn across the approach. Its
playtest tests pin the accumulator (a 30 ms frame must not lose physics, a two-second
hitch must not teleport the player), that holding right
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

# Findings and deviations from the plan

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

   *Half of this stopped being true in finding 44.* L1 is still free — an initial
   population wins it — but L2 no longer is: with jumping edge-triggered (finding 43),
   crossing its pit takes a timed press rather than a held button, so only five of twelve
   seeds contain a genome that wins it at generation 0 and the rest need one to five
   generations. L2 is now the pack's first *earned* level, which is what it should always
   have been; the sentence above is kept because the reasoning it records — that a free
   level is a sign-bug guard and not evidence of learning — is what the difficulty test
   was built on.
4. **Jumps fire while the button is *held*, not on a press edge.** With the
   action table re-queried every 4 ticks, an edge trigger forces the network to
   discover a release-then-press rhythm whose only effect is to make jumping hard
   to evolve. Holding through a landing simply re-jumps.

   ***Reversed in v2 (finding 43), deliberately and with the opposite conclusion.***
   A jump now needs a fresh press, because holding the button produced a perfectly
   periodic hop with no decision in it — measured at 2.58 jumps per 100 frames, which is
   the rate of the button rather than a policy. The release-then-press rhythm turned out
   not to be "hard to evolve" but to be exactly the *thing worth evolving*, and the press
   buffer is what keeps it learnable. The history matters because the v1 reasoning above
   was not stupid: it was right about the cost and blind to what the cost was buying.
5. **A jump is force-held for 8 ticks** (`min_jump_hold`), so a 4-tick action
   repeat cannot shrink every jump to a 1-tile hop. Shortest jump is 1.70 tiles,
   full is 2.375.

   ***The hold is one block now (finding 46), and the reasoning above is exactly why.***
   Eight ticks is *two* blocks of the agent's cadence, so the first release it could ask for
   was ignored: the ladder was `{1.70, 1.70, …}` and the "1-tile hop" this note is careful
   about was in fact unreachable — holding for one block and for two produced the same jump.
   The floor has to be exactly one block, which gives `{1.155, 1.70, 2.085, 2.31, 2.375}`:
   shorter than this note's 1.70 *and* aimable.
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

   *Re-measured after the movement pass, the pair is L1 164 frames (2.6 s) at 8× and
   27 (0.43 s) at 50×, L3 177 (2.8 s) and 30 (0.48 s)* — the old two-to-one spread is
   gone and L3 now costs marginally more than L1. The honest reading is that
   generation-1 *behaviour* changed, not the levels: the explanation above was about
   agents surviving versus dying, and the numbers no longer support it. The current
   table is in [What the ladder costs to watch](#what-the-ladder-costs-to-watch).
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
   the tests were being written. And the edit that used to be refused up front is now
   allowed, with its consequence stated instead: the eraser may leave a document with
   no flag, which the writer still refuses and the verdict line names before Save is
   ever pressed. What a resize may not do is *lose* the flag — a shrinking grid drags
   its goal back in — because a flag the user did not delete is not the resize's to
   drop.

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
   from the vermilion body and **224** from the rim. The two halves take turns there as well
   as anywhere: `(55, 64, 80)` of deep rock is 201 from the body but only 60 from the rim, and
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
    the shipped pack and requires **every prop family the table can name to be dressed
    onto them** (`every_prop_family_is_dressed_onto_the_shipped_pack`); restored to the old
    roll it fails with "no flowers is drawn anywhere in the shipped levels", naming the
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
    all, where the level's bottom edge has to do — plus every shipped level by *property*: every
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

43. **The population was living on a free bunny-hop, and the fitness had nothing to say
    about it.** Jumping was *level*-triggered: the action table is re-queried every 4 ticks
    and a held button re-jumped on every landing, so "hold jump" was a perfectly periodic
    hop with no decision in it — measured at 2.58 jumps per 100 frames, which is the
    mechanical rate of the button, not a policy. Two changes, one on each side of the
    boundary: the sim now makes a jump need a **fresh press** (with a 6-tick press buffer,
    so deliberate jumping stays forgiving — a press a few ticks before touchdown still
    fires), and fitness v2 charges `-2.0` **per jump taken**, which is exactly what the
    jump's own 40 frames of airtime would have earned. A held button now jumps once per
    episode where it used to jump fourteen or fifteen times, and the physics test says so:
    `a_held_jump_jumps_once_and_a_fresh_press_jumps_again`.

    The term is deliberately small. The cheapest jump that *does* something — clearing a
    one-tile gap — earns 16.0, eight times its charge, so it can never teach a population
    not to jump when jumping is the answer; its job is the case where the old landscape had
    nothing to say at all, because `run` and `run_jump` reached the flag in the same number
    of frames for the same progress. **And the champions still hop**: measured on the seed-1
    champions of all eight levels, they take 2, 13, 13, 12, 8, 6, 4 and 11 jumps. What
    changed is not the number of jumps but their *kind* — each of those is now a press the
    network chose, and no held button can produce the pattern — and it is fair to say the
    remaining hopping is a tie-break the fitness declines to win rather than a decision it
    rewards. `-2.0` is nothing beside a 10 000-point goal bonus, and whether the rest of
    the hopping is load-bearing on the pit levels or simply unpunished is the thing this
    pass did not settle.

    **Two abilities came with it.** *Sprint* is a raised speed cap that ramps in: 0.10 →
    0.15 tiles/tick at 0.005/tick², so it costs 1.125 tiles of runway, and a sprint jump
    covers 6.00 tiles where a running jump covers 4.00 — which the level designer is told
    in exactly those units, because that pair is what a pit is designed against
    (`a_sprint_jump_clears_a_gap_a_running_jump_falls_into`). *The wall climb* is the Super
    Meat Boy rule: press into a wall while airborne and the body clings and slides at 0.06
    tiles/tick, and a jump there launches *away from the wall* — no separate button,
    because the verb does not change. A wall jump is worth exactly one jump's height
    (`wall_jump_v0 == jump_v0`), which is a deliberate choice and a pinned one: a shaft
    climb then costs no extra height arithmetic, and the reachability oracle can treat the
    two as the same window. The ability is discoverable rather than documented — running
    into a wall is a dead end, and the only thing left to try there is jump.

    **An agent cannot reason about a move it cannot see or feel**, so both abilities are
    sensor changes, not physics-only ones: v2 appends `wall_dir` (the side the body is
    pressed into), `clinging`, and `sprint_charge`, and renormalises `vx` by `sprint_speed`,
    because the old normaliser saturated at 1.0 for a running body — which would have made
    sprinting and running *indistinguishable*, the one thing a speed sensor exists to tell
    apart. Running now reads 0.67 and sprinting 1.0. The action table grows from six to
    nine (`sprint`, `sprint_jump`, `retreat_jump`; a leftward jump is what makes a wall
    climbable from the keyboard too), and the network to 93 → 16 → 9, 1657 genes. That is
    the version trio's first real workout — sensors **v2**, fitness **v2**, workspace
    **0.2.0** — and every champion trained before it is refused, for the right reason and by
    a test (see [The version trio](#the-version-trio)).

44. **The movement pass moved the difficulty ladder, and the oracle proves it was not the
    new gear that did it.** The pack was re-measured end to end (twelve seeds, pop 200, 420
    generations): L3's median went 11 → 20, L4's 16 → 9, L5's 68 → 14.5, L6's 40 → 8.5, and
    every level is now solved on all twelve seeds where L5 and L6 used to miss one or two.
    L2 also lost its free win — the same population that crossed its pit by holding the jump
    button now has to decide — which is the ladder moving for a real reason, and the pack is
    better for it: five of twelve seeds still contain a genome that wins it at generation 0,
    and the rest need one to five generations.

    It would be easy to read that as "the new abilities made the pack easy", and the
    reachability oracle says otherwise. Run with an ability *withheld* — `ReachOptions { wall_climb: false }`,
    `{ sprint: false }` — L1–L6 stay reachable with either one disabled; only L7 becomes
    unreachable without wall jumps and L8 without sprint. So the new moves are not
    *required* by L4–L6, which rules out "the levels now need the gear" but not "the gear
    made the timing easier". What the champions *do* is the better clue, and it is
    measurable: replaying the seed-1 champion of each level and counting how its 4-tick
    decision slots are spent, L1 spends 156 slots on `sprint` (a flat level, where
    sprinting is only faster), L5 spends 62 on `sprint_jump` and 40 on `sprint` over its two
    four-tile pits, L6 60, L8 43, and L7 36 plus two ticks actually clinging to a wall. The pits are crossed *sprinting*, and a
    four-tile pit is a knife-edge for a 4.00-tile running jump and a comfortable one for a
    6.00-tile sprint jump. The oracle is right that sprint is not **required**; the ladder
    measures the second thing, and "crossable" and "comfortably crossable" are different
    questions. That is the honest version of "the abilities flattened the ladder": the
    levels did not get easier for a structural reason, they got easier for a control reason,
    and the two ability levels are the only ones where the move is the level.

    **Both ability levels are new, and each is one decision.** *L7-ascent* is two
    three-tile wall climbs: a full jump reaches 2.375 tiles, so neither ledge can be jumped
    onto and each one is a wall jump off its left face — the face is a dead end for a policy
    that just runs right, which is what makes the move discoverable. *L8-long-jump* is two
    five-tile pits, which is inside a sprint jump's 6.00 and outside a running jump's 4.00,
    with far more than the 1.125 tiles of runway sprint needs on each side: the level is the
    same decision made twice, and the second one has to be made while the first is still
    being survived (a walker patrols the middle landing). Both are pinned in
    `crates/sim/tests/levels.rs` and in the difficulty table, and both were measured to fall
    to evolution rather than to luck — L7 in 12.5 generations on the median, L8 in 9.5.

    **The pits themselves stayed lethal, and that is measured too.** Every pit in the pack
    is a gap in the bottom two rows of the grid, and the most generous recovery a policy can
    make — press back into the wall the body just fell off — never reaches it: the body
    leaves the lip at running speed and is out of the world 107 frames later with zero cling
    ticks (`walking_into_a_pit_is_still_fatal`). The window that *does* exist is the jump:
    a body that jumps at the lip and turns around in the air lands back on the ledge it
    jumped from, in one jump and no cling
    (`a_body_that_jumps_off_the_lip_can_turn_around_and_land_back`). So recovering from a
    pit is a decision made at the edge rather than a rescue performed in the shaft, which is
    exactly the difficulty the pack's pit levels are made of. Both tests are the same
    fixture, so the only difference between death and recovery is the authority the agent
    takes at the lip.

45. **The wall jump is discovered in three generations and *used* in twenty-five: the
    expensive part is the second one.** The owner's report was "60 generations and it still
    cannot wall jump", and the measurement says the move is not what is missing. Replaying
    every fourth genome of L7-ascent's population each generation at pop 200:

    * the cling and the wall jump are there almost immediately — **at generation 0, 4 to 17
      of the 50 sampled genomes are clinging to the first wall and 2 to 8 of them have
      wall-jumped off it**;
    * by generation 3 the best agent is standing on ledge 1 at `x ≈ 23.4`, so it has climbed
      the three tiles once;
    * and then it sits at that `x` for 25 to 90 generations, because the second ledge is a
      *second* discovery — the same climb, 2 tiles further right, from a different standing
      state. On seed 12 the population's best does not move for **89 generations** (the
      stall line in [The M3 problem](#the-m3-problem) is that run).

    Generation count is therefore the wrong knob; **population size** is the real one. L7's
    first win by seed, measured at four sizes: pop 200 → `29 31 7 25 11 14 44 4` (all eight
    seeds inside 44, and the twelve-seed ladder above has one seed at 90); pop 128 → up to
    84; pop 64 → one seed unsolved inside 200; pop 32 → three of eight unsolved, one of them
    at generation 174. A population that small simply does not contain enough variation to
    stumble onto the move *and* keep it.

    *The wall school exists for this.* `levels/l7-wall-school.json` is the same climb with
    nothing else in it: a three-tile plateau with the flag on its top, so the move pays the
    instant it happens, and walking into the face is a dead end (the oracle refuses it with
    wall jumps disabled, like its sibling). It learns in a median of **2.5 generations**
    (0–8 over twelve seeds, pop 200 — one seed in twelve gets it at generation 0) against
    the ascent's 12.5, and it is the level to train *before* the ascent rather than beside it.

    **The obvious fix does not work, and that is the part worth writing down.** Pointing the
    stall chain at the school — `watch --level levels/l7-ascent.json --curriculum
    levels/l7-wall-school.json` — fires correctly, hands the stuck population 40 generations
    of the school, and the ascent gets *worse*: the population comes back as a school
    population and L7's own best fitness falls from ~370 to ~150, which loses the climb it
    had already learned. L7 seed 1, solved at generation 29 when left alone, does not solve
    within 150 generations with the excursion. A curriculum step is worth taking only when
    the fallback level's skill *transfers*, and this is the measured counterexample to
    assuming that it does: a level swap spends the population's progress on the level it was
    making it on. The school teaches the move fastest; it does not rescue a stuck ascent.

46. **The jump ladder had a dead rung: one block of hold and two blocks produced the same
    jump.** The owner's report was a level that "only had to jump between spikes" on which
    the agents "repeatedly jumped into them — it only ever jumps at the full length, it
    doesn't seem to jump half lengths". The height half of that was a real defect in a
    single constant. `min_jump_hold` was 8 ticks — **two** decision blocks — while the
    network is asked once per block and its answer is held for the whole block, so
    releasing the jump at the first opportunity the agent can actually ask for still
    produced an 8-tick hold. Measured, the ladder was `{1.70, 1.70, 2.085, 2.31, 2.375}`:
    block one of hold bought nothing at all, and the shortest jump the agent could ask for
    was not a jump it could ask for. At one block the same sweep gives
    `{1.155, 1.70, 2.085, 2.31, 2.375}` — five gears, one per decision spent, which is what
    "it doesn't jump half lengths" was describing.

    `min_jump_hold` is `DECISION_TICKS` now and the equality is asserted
    (`physics_bible_derived_quantities_match_the_plan`), with the ladder pinned rung by rung
    in `every_extra_decision_block_of_hold_buys_height`. The change also invalidated a
    fixture built on the old airtime, which is worth recording because it is the failure mode
    of a test keyed to a trajectory: `a_press_just_before_touchdown_is_remembered_and_an_old_one_is_not`
    pressed jump twenty ticks before landing, which was far from the landing only while the
    shortest jump was two blocks long. At one block that press arrived *inside* the 6-tick
    press buffer and correctly fired, so the test failed while the behaviour was right. It
    presses two ticks after take-off now — derived from the shape of a jump rather than from
    a trajectory — and its message names the buffer.

47. **`brake` was inert in the air, so every hop the agent owned overshot the pad.** This is
    the length half of the same report, and it was a hole rather than a tuning problem. A
    jump's carry is `speed × airtime`, and of the nine macro-actions exactly one — `brake` —
    exists to stop moving. The friction branch picked `brake_friction` only when the body was
    **grounded** and `air_friction` otherwise, so in mid-flight `brake` was *bit-identical to
    `walk`*: the one "stop" button in the table did nothing at precisely the moment a hop's
    length is decided. Measured at sprint speed, carry per block of hold was
    `2.83, 3.49, 4.09, …` — the shortest hop in the game carried 2.83 tiles, and a body 0.6
    tiles wide cannot stand on the pad such a hop lands on. With the air brake the same sweep
    is `0.90, 1.50, 2.10, 2.70, …` up to the 6.00-tile sprint jump: **length is a knob with
    a rung per block of hold**, which is what makes a hazard *ladder* rather than a hazard
    *choice* something the agent can be asked to learn.

    Pinned by `braking_in_the_air_is_what_makes_a_short_hop_expressible`, which derives the
    landing window from the geometry rather than hard-coding it — a hazard tile to clear plus
    a pad `player_w` narrower than a tile, so `[2.0, 2.4]` tiles from take-off — asserts that
    *no* coasted hop fits it at any length of hold, that a braked one does, that carry is
    strictly increasing in hold, and that the top of the ladder is still the full sprint
    jump. End to end, on the shape that produced the report (one-tile spikes, one safe tile
    between them): a beam search over action sequences could only cross such a floor with a
    124-decision overshoot-brake-reverse dance, and the population's best agent sat at
    `x = 7.4` — before the first spike — for 1 760 frames. It now **solves at generation 110**
    at pop 1000 (seed 3, 15 jumps, 625 frames), and its two-safe-tile sibling solves in 26–43
    generations at pop 500. Two honest limits: the same budget still fails on seeds 1 and 2,
    which is the level's business rather than the physics'; and the reachability oracle models
    no air brake and cannot see a hazard sitting *on* a platform, so on a floor of spikes its
    "beatable" is a statement about the floor.

48. **`fitness v3`: the per-frame survival term was the park trap's other half, and it is
    gone.** Finding 15 named the trap; this is the arithmetic that closes it. Crossing a
    hazard was worth about two tiles of frontier (32) against a death priced at 120, so
    attempting paid only above a **79%** success rate — and the survival bonus paid 88 on top
    of that for standing still. The measured best of 4 800 evolved agents, on a flat floor
    with one-tile spikes and one safe tile between them, walked to `x = 7.4` — the last tile
    before the first spike — jumped once and stood there for 1 760 frames, scoring 169 of
    which 88 was survival. v3 deletes the per-frame term and prices a death at 1.25 tiles
    (`DEATH_PENALTY = -20`), which turns the same calculation over at roughly **one hazard in
    four**. Nothing in v3 pays for dying and nothing pays for standing still: progress is
    still the rightmost ground *ever* reached, so a death forfeits the progress it failed to
    make and can never bank any, and the option v3 removes is the one that paid for *not
    playing*. `FITNESS_VERSION` is 3, the argument, the arithmetic and the
    `clearing_a_hazard_and_dying_beats_parking_in_front_of_it` assertion live in the
    `evo::fitness` module docs, and on that fixture the population *attempts* the floor now
    instead of parking in front of it.

49. **A leap that ends in the air keeps its distance but loses its landing: checkpoints, and
    the half of the park trap the frontier could not reach.** Finding 15 called the failure a
    cul-de-sac — a pit gives the population no feedback until a jump succeeds — and
    proposes "intermediate checkpoints (a safe landing zone beyond an obstacle)" as the fix.
    `fitness v4` implements it, and the design question was *what a checkpoint is*, because
    the obvious answer is a relabelling of distance covered and would be a proxy reward.

    The frontier is credited for **any** x the body reaches. That includes the airspace over
    a pit it is about to fall into, so a failed leap already keeps the ground it covered;
    what it cannot keep is the landing. So the checkpoint is defined as the strict half:
    the sim counts a **safe landing zone the body stood on**, once, and only while
    `player.grounded` is true. A genome that sails over a hazard and dies beyond it scores
    the frontier and nothing else; the same leap that arrives scores both. The counting
    lives in the sim (`Sim::step`) because the score has no level to ask — and because the
    condition is a fact about the trajectory, not about the level's shape.

    Three definitions had to be got right, and each was measured rather than guessed:

    * **A zone, not a tile.** A body that clears a hazard and touches down anywhere in the
      run beyond it has arrived; paying only for the first safe tile would refuse the same
      crossing for landing one tile further on, making the reward a function of landing
      precision rather than of the crossing. (`Level::landing_zone`.)
    * **A hazard or a real drop, not a step.** The first version counted any zone the body
      could not have *walked* into, which on the shipped pack made `l2-first-gap` pay for
      four landings where it has one pit and `l7-ascent` for three where it has two ledges —
      the extras being steps up onto raised platforms. The rule now needs the column left of
      the zone to be a hazard at standing height, or to have **no ground for two rows**, so a
      one-tile step down is excused and a fall is not. Re-measured: `l1-flat-run` 0,
      `l2-first-gap` 1, `l8-long-jump` 2 (its two pit ledges), the spike floors one per
      hazard. A flat level cannot collect a single point of it, which is what leaves the
      flat levels' landscape untouched.
    * **Never the spawn ledge.** The zone touching the level's left edge is not a checkpoint:
      nothing was crossed to reach it, and it would hand every episode a free credit.

    Sized at **one tile of progress** (`16.0`). It has to be legible beside the crossing
    that earns it — the smallest hazard crossing is worth `2 × 16 = 32` of frontier — and
    stay far under the frontier itself so it cannot become a second objective: a
    thirteen-hazard floor banks 208 points of it against a 10 000-point goal. It is one-shot
    per zone, so it cannot be farmed, and it cannot be banked by dying: standing on a
    checkpoint is worth exactly the two tiles of frontier under it, no more
    (`checkpoints_cannot_be_farmed_in_place_of_progress`).

    **Measured** on the tightest hazard floor in the repo (one-tile spikes, one safe tile
    between them — the shape the owner's report produced, and one that a frontier-only
    fitness never solved at any population up to 1000): three seeds per budget, v3 versus v4.

    | budget | v3, frontier only | v4, with checkpoints |
    |---|---|---|
    | pop 200 × 120 generations | unsolved — `best_x` 11.8 / 13.3 / 11.8 | **solved at generation 107** (seed 2); 12.5 / 13.4 otherwise |
    | pop 500 × 400 generations | unsolved — 11.9 / 11.9 / 11.9 | unsolved, but 12.6 / 18.9 / **35.4** |
    | pop 1000 × 800 generations | solved once, generation 110 of 800 | solved at generation 313 (seed 2); 12.8 / 14.6 otherwise |

    The middle row is the finding. v3's plateau was **one** hazard crossed — it stalled at
    the first spike for four hundred generations on every seed, which is the cul-de-sac as a
    number — and v4's is **thirteen of fifteen**, with the winning genome finishing the floor
    at 17 jumps and 15 checkpoints. The first row is the other half of it: a level that needed
    a population of 1000 to fall now falls at 200.

    Two honest limits, because that table invites over-reading. The *easier* hazard floor
    (two safe tiles between spikes) is a **wash** — v3 solved it in 14–69 generations over
    six budgets, v4 in 18–50 — which is what a term that fills in a missing gradient should
    look like where the gradient was not missing. And the tight floor is still a lottery at
    these budgets either way; the checkpoint term prices the lottery better, it does not make
    it a cert. Nothing about the shipped ladder    weakened: `crates/evo/tests/difficulty.rs` still passes unchanged, and the hazard-free
    levels have no checkpoint to earn — the arithmetic, not a measurement, is why they
    cannot be affected.

50. **A stalled run practises its *next move*: the run's own level from the furthest stand
    its best genome made, with the flag moved to the stand just past it.** Findings 19–22
    say what a stalled run does — checkpoint the stuck population, take a detour to an
    easier level, bring the population back — but the chain needs an easier level, and a
    level somebody drew by hand is not always there. This derives one from the run's own
    landscape, and the derivation had to get three things right before it worked. Each is
    a lesson with a measurement behind it.

    **Where the run is: a stand, not an apex.** The obvious reading of "how far the run
    has gotten" is `closest_approach`, and it is wrong: it is the running minimum over
    every *state* the body passed through, including the apex of a leap it then fell back
    from. On a zig-zag climb one free wall-jump apex outranked the next four platforms of
    the route, so arriving on any of them scored no progress and selection preferred the
    body that stayed where it was. `stuck_point` reads the body's distance on the frames
    it is **grounded** — the furthest it has actually *stood* — and the practice spawn is
    snapped to ground the body can stand on (a body at a ledge's edge overhangs its
    support, and a spawn dropped into the gap is refused).

    **Where the flag goes: the next stand, not the level's end.** The first version moved
    only the spawn and left the flag where it was. Measured on the zig-zag tower
    (`levels/scratch-spike-tower.json`, 14×40: a zig-zag of two-tile platforms over
    spikes, flag at the top — the level this work was measured on), it failed:

    | practice level | solved (12 seeds × 420 gen) | checkpoints banked | what the detour did |
    |---|---|---|---|
    | none (the chain off) | 0/12 | 4–12 | — |
    | spawn moved, flag left at the level's end | 0/12 | 4–7 | learned at the spot (best there 83→230) and lost the climb |
    | spawn moved, **flag at the next stand** | **1/12** — first win at generation 256 | 3–18 | 53 of 57 detours reached their flag; the practice chain walked up the tower |

    The middle row's mechanism is worth stating exactly, because it is invisible in the
    champion: with the flag at the level's own end a practice episode can only bank the
    *tail* of the run's score — the stuck population scores ~294 on the tower from the
    bottom and its practice episodes could reach at most ~230 from the moved spawn. The
    detour is a two-level suite `[practice, main]` ranked by the **worst** level (the
    "hold both" rule, and the only ranking that should preserve the climb), but with the
    tail always the worst, the minimum *was* the tail: every detour bred the climb away.
    The run's own chart says it — the maximum fell 295→115 the moment the detour started
    and was still 273 at generation 799, *below* the population the detour took out. It
    also hides behind `best-ever`: the champion is banked before the detour, so the run
    looks unchanged while the population under it is worse.

    Moving the flag to the stand just past the stuck one fixes the scale at the root.
    Winning the practice level is worth the same 10 000 as winning the level, so a genome
    that makes the move is ranked by its *climb*, and a genome that keeps the climb but
    loses the move falls under the move's score — the minimum now means "hold both" where
    both can score. It also makes `flag reached at its generation N` mean the move is
    *owned*, which is the reading the note needs. With the step re-armed on each new stall
    spell (the authored `--curriculum` path keeps its one-step-per-run rule), one run
    works through the tower move by move: the winning seed practised (12,31)→(10,29),
    (10,29)→(7,27), (4,25)→(1,23) and finally (9,1)→(12,1) — the move right before the
    top flag — and won the level at generation 256. Reproducible from the seed.

    **What it is claimed to do.** 1 of 12 at 420 generations is a rescue, not a solution:
    the tower still falls to this budget only occasionally. The claim is the mechanism —
    the detour learns the move, keeps the climb, and hands back a population that is
    better on its own level (best-evidence fitness 310–719 against the baseline's
    282–612, and the two best runs banked 17–18 of the tower's checkpoints against the
    baseline's 12).    One honest limit: `next_stand` picks the nearest stand genuinely
    closer to the flag and does *not* check reachability, so a stand the body cannot
    actually reach makes the practice level unwinnable — visible as "flag not reached
    there yet", which is what 4 of the 57 detours said. *(Corrected in finding 51: those
    four misses were the level's hard crossing won ten times over in the same arm, not an
    unreachable stand — and the proximity guess this paragraph names was the real defect,
    in two measured forms.)*

    Shipped as `evo::curriculum`: `stuck_point`, `next_stand`, `practice_level`,
    `practice_step` (the two-level suite step), `save_practice_level`
    (`<level>.population.json` → `<level>.practice.json`, so the derived landscape is a
    file somebody can open and play). Wired in `watch`, `train` and the window's evolve
    worker.

51. **The next stand is the route's next crossing. Proximity was measurably wrong.**
    Finding 50 placed the practice flag on "the nearest stand that reads closer to the
    flag" — a straight-line guess where a route question was being asked. The tower arm
    it was measured on contains both ways that guess fails, in its own detour log:

    - `(4,9) → (6,11)` practised **backwards**. The run stood on the leg's stand at row
      10 and the flag went to the stand at row 12 — the one it had just climbed from. A
      zig-zag turns its back on the flag for a whole leg, so the stand *behind* reads
      closer to the flag than the stand the route visits next, and the detour duly won
      its flag (backtracking is easy) and spent 40 generations owning a move the run
      already owned.
    - `(7,11) → (4,5)` ordered a practice episode **three crossings long**. The stands
      up the leg (rows 10 and 8) read *farther* from the flag than the stand at row 12,
      so the "closer to the flag" filter deleted them and the nearest survivor was three
      jumps away — the whole chain in miniature, exactly what a derived level exists to
      avoid.

    Finding 50's closing limit was a **misdiagnosis**, correctable from the same log:
    all four "flag not reached there yet" detours were the crossing (10,29)→(7,27),
    which the same arm won ten times. The level's hard move is hard; the stand was never
    unreachable. The reachability hazard was real but latent — proximity could pick a
    stand no move reaches, and nothing would notice until the practice level went
    unwinnable.

    **The fix: distance is a graph question.** `sim::reach::goal_routes` builds the
    move graph and each stand's hop count to the flag: landing zones are the nodes (a
    spike at standing height splits one platform into two stands the body cannot walk
    between — `extract_platforms` is too coarse to be a stand), and every edge is one
    canonical crossing from `edges_from`, so the route can only ever claim a move the
    game has. `next_stand` takes one crossing off the stuck stand whose hop count to the
    flag does not grow: strictly fewer is route progress, the same count is a stepping
    stone (the near ledge of a wide pit — the nearest forward move, not the next-but-five,
    finding 50's decomposition rule preserved), and *more* is the run being sent back.
    The straight line survives only as a tie-break within one rung of the graph. A stand
    with no route to the flag has no next stand at all, and the run says so instead of
    ordering an impossible jump.

    **The measurement.** 12 seeds × 420 generations on the tower, same protocol as
    finding 50's arm:

    | practice flag | solved | detours reaching their flag | detour shape |
    |---|---|---|---|
    | nearest stand reading closer (finding 50) | 1/12 — win at gen 256 | 53 of 57 | 2 of 57 wrong: 1 backwards, 1 three crossings |
    | **the route's next crossing** | 1/12 — win at gen 299 | 96 of 104 | every one a single route hop |

    The outcome is the same within noise and the derivation is not. Every practice pair
    in the new arm is an adjacent leg of the zig-zag — (13,31)→(10,29), (10,29)→(7,27),
    (7,27)→(4,25), (4,25)→(1,23), (1,23)→(3,21), (6,19)→(9,17), (9,17)→(12,15),
    (13,15)→(10,13), and (4,9)→(1,7) where the old rule practised backwards — and every
    seed stepped back about twice as often (104 vs 57 detours: 2→4, 1→2, 5→8 …). The
    re-arm gate opens only when the run sets a new best after a detour, so the count is
    the read-out of how often the chain came back and the run moved again; a detour that
    learns nothing shuts the gate and the run gets no second step back until evolution
    alone breaks its record. Champion heights are unchanged (3–19 of the tower's
    checkpoint zones banked against 3–18), seed 11 won both arms (gen 299 against 256,
    reproducible from the seed), and the eight misses are all the one genuinely hard
    crossing — (10,29)→(7,27), 28 attempts, 8 unlearned in 40 generations.

    The claim is the mechanism, not the tower's win rate: the practice episode is now
    always the crossing it claims to be, always one the game has, and never the ground
    behind the run. What the tower does with that at 420 generations is a separate
    question — its binding constraint is the crossing at rows 30→28, and no ordering of
    practice flags makes that jump shorter.

    Shipped as `sim::reach::goal_routes` (`Trajectory` gained `landed_at`, the column a
    body is supported by, so a landing maps to the stand it reached) and the rewritten
    `evo::curriculum::next_stand`; the practice-level contract and every front-end are
    unchanged.

52. **A move cannot start inside a hazard. The oracle's one false positive hid a sealed
    level — and the stall chain refused to say so.**

    The reachability oracle's hazard check runs *after* movement — the game's own rule:
    a body dies the tick its box overlaps a spike. But a take-off sweep authors the body
    standing at an exact x, and where a platform's column is spike-topped, some of those
    x put the box *inside the spike at tick zero*. Such a body leaves the tile between
    tick zero and tick one and is never checked at all: the sweep invented jumps out of
    a spot no body can occupy (to stand there the body would have had to walk into the
    spike first). `simulate_jump_at` and `simulate_wall_climb` now refuse such a start
    (`hazard_start`): a death at the take-off, and no edge.

    What it hid: a hand-built 8×30 spike tower (drawn over `levels/l1-flat-run.json`)
    that four phantom edges reported as beatable. In fact one spike — at (4,12) —
    seals it. The ledge at (3..4,13) is the only take-off toward the shelf at (6..7,11),
    and the spike's death window covers every arc across it: the body must have risen
    1.14+ tiles before its box reaches column 4 (air accel is 0.006/tick², so x stays
    ≤ 3.1 for the ~5 ticks that takes), and it must then clear x ∈ (3.1, 5.0) while
    still above the window — the descent re-enters it before the drift can pay for
    itself. The escape up the right wall is guarded the same way by the spike at (7,16).
    Evidence: 864 systematic take-off schedules (delayed input, carried speed, held
    hover) and ~260 000 random 200-tick action sequences from *every* stand on the
    sealed side — zero ever bridged to the reachable set or touched the flag. Deleting
    the spike, or moving it one tile right to (5,12), flips the whole level to `OK`
    (16 of 18 platforms reachable, goal reached). Learning cannot beat geometry; the
    oracle now says so instead of agreeing with the drawing.

    The second half is the silence. When the practice derivation found no crossing
    toward the flag — exactly what this level produces at the stuck spot — all three
    front-ends `continue`d without a word, which is indistinguishable from a dead stall
    chain. `practice_level_for_stall` now returns `Err(reason)` and `train`, `watch` and
    the lab say it once per stall spell: *no crossing toward the flag from tile (…);
    the run stays where it is (…)*. A run stalled on an unbeatable level now says so on
    its own.

    Pinned by `a_move_cannot_start_inside_a_hazard_and_a_sealed_route_stays_sealed`
    (a miniature sealed cut: the phantom take-off dies at tick zero, the far stand still
    reaches the flag in one real jump, and the near stand has no route at all) and by
    `every_shipped_level_is_reachable_in_principle`, which now reports the redrawn level
    as unreachable — the honest verdict the oracle owed the pack.

53. **Jump-timing seeds in the practice detour: built as asked, measured flat — the
    detour was never the crossing's bottleneck.**

    The request was to make the practice detour teach a hard crossing: when the run
    stalls on a hop, seed the detour population with jump-timing mutations around the
    stuck genome's attempts. What that means in this representation: the policy is an
    argmax over eleven action scores, so a jump begins on the tick the jump actions
    out-score the moving ones — a boundary in output space. `genome::jump_timing_slots`
    names the genes that draw it (the output layer's row and bias for every action that
    holds jump), and `genome::mutate_jump_timing` is the fine step over them alone: one
    to three genes at the run's own sigma, every other gene copied exactly. The practice
    detour's weakest slots (a quarter of the population ceiling, eight at pop 200 —
    [`curriculum::PRACTICE_TIMING_SEEDS`]) are replaced by such children of the best-ever
    genome before the first generation on the practice level, ids fresh, draws from their
    own stream so the trainer's RNG — every paired reading in this file depends on it —
    is untouched. [`curriculum_step`] does not seed: an authored easier level is a whole
    landscape to re-learn, not one move to polish.

    Measured on finding 51's protocol, which makes the comparison paired: 12 seeds × 420
    generations on `scratch-spike-tower`, the recorded `out/tower-route` arm as control,
    and the two arms bit-identical up to each detour (verified: their chart rows differ
    only in the wall-clock column). The new arm is `out/tower-timing`.

    | reading | control | with timing seeds |
    |---|---|---|
    | hard hop (10,29)→(7,27): detours reaching the practice flag | 10 of 13 | 12 of 16 |
    | hard hop: generations to that flag (median of those) | 6.5 | 6 |
    | all detours reaching their flag | 48 of 51 | 45 of 49 |
    | wins | 1 of 12 (seed 11, gen 299) | 0 of 12 |
    | champion height (checkpoints banked, mean of 12) | 7.6 | 7.1 |

    Flat on every reading — and the paired per-seed tallies are noise (heights: five up,
    four down, three ties). The mechanism is kept because it is the behaviour that was
    asked for and costs nothing measurable; `PRACTICE_TIMING_SEEDS = 0` is an off arm in
    one edit.

    What the measurement actually corrects is the premise. The practice detour does not
    plateau on a crossing: whenever it learns the hop it learns it in a **median of two
    generations** (six on the hard hop) — the population is dropped at the stuck stand
    with a flag one move away, and the near-miss genome is one mutation from making it.
    The misses — three or four detours per arm on the hard hop, ~25% — are detours where
    the population cannot make the hop at all inside forty generations, and eight timing
    seeds at four percent of a converged population do not rescue those. The tower's
    binding constraint is unchanged and is still finding 51's: the hop that is *hard to
    transfer*, rows 30→28, where even a detour that reached its flag leaves the run
    failing the same crossing. If the crossing learning is to improve, the place to look
    is the misses' stall state — what that population cannot do at all — not the timing of
    a near-miss it already polishes in two generations.

54. **The window looked like a prototype because its controls *were* egui's: a bar of stock
    buttons is a tool, and the fix is one vocabulary for everything the pointer can press.**

    The report was "it looks and feels more like a prototype than a game", and the diagnosis is
    not a matter of taste: every control in the window was `ui.button`, `ui.selectable_value` or
    a hand-painted rectangle, so the furniture was egui's defaults wearing the theme's colours — a
    rounded grey rectangle with a text label, four times across the top. What a game's tool bar
    *is*, and what none of that is, is a plate of plastic in a socket: a lit top edge, a shaded
    bottom one, a face that moves when you push it, and the game's own art on the button.

    The vocabulary now lives in `crates/app/src/ui.rs` and it is deliberately *pure where it can
    be*, because a look that can only be judged in a screenshot is a look that regresses silently.
    `bevel(PlateState, Rect) -> Bevel` takes one control's state and rectangle and returns every
    colour and sub-rectangle it needs, with no `Ui`, painter or input involved: what "tactile"
    means is then a claim a test can make — a pressed plate's content sits exactly `PRESS_SINK`
    (2 points) below a resting one's while its *face* stays put, a pressed plate inverts its edges
    (shadow above, light below — an inset face), a disabled plate ignores the pointer entirely
    rather than drawing hover feedback for a press it will not answer, and the five states have
    five different faces with the selected one wearing the accent and dark type on it.

    Three rules came out of it that are worth keeping, because each one was a bug first.

    * **A sign is two lines, and a `Frame` does not know that.** The level's name went on a
      plaque beside the tools, and the first rendered frame showed the name and the file laid out
      *side by side* on one line: a `Frame` inside a horizontal bar inherits the bar's layout. The
      fix is one `with_layout(top_down)` and it is load-bearing — the one-line version was 235
      points wide against the 140 the sign needs, which is a control's worth of dock spent on a
      caption.
    * **A reading that does not fit is not drawn, it is measured.** The run's live line is the
      only thing on the bar that changes on its own, and the bar has to hold a sign, five tools, a
      four-cell switch and a *Fit* inside a window that can be dragged to 900 points. `banner`
      therefore takes the room it has been given, lays its text out, and returns `None` — drawing
      nothing at all — when the sign would not fit. The alternative is a banner cut in half by a
      clip rectangle, which reads as a broken window rather than as a smaller one.
    * **A label that does not fit is a bug, not a wrap.** A switch cell is a fixed box, and its
      word is not wrapped, so a cell too narrow for its word clips the word. `segment_label_room()`
      is shared by the widget and by a test that measures the longest word the switch has
      (*Evolve*) in the font it is drawn in, and the widget itself drops the icon when the word
      would not fit beside it — the switch must always be able to name its four modes.

    The bar itself is worth stating as a design rather than a list: the level's name chalked on a
    sign (file and unsaved dot under it), the five document tools, the mode switch as *one object*
    (a socket holding four exclusive cells, the chosen one standing out — four independent plates
    would say four things that could all be on at once), *Restart* only while a person is playing,
    *Fit*, and the run's own numbers at the far end with a lamp that is green on a win, amber on a
    stall and gold while the search is still improving. Every icon is an atlas sprite, so the tool
    bar is made of the same sixteen-pixel art as the level behind it. The palette's brush cells and
    its scenery picker go through the same `bevel`, which also removed four hand-written colour
    literals from the picker — including the accent's own gold spelled as `250, 214, 102`.

    No animation, on purpose: a plate's state is a fact about where the pointer is, and a
    spring-damped bounce at forty points reads as a web page rather than as a game.

    Pinned by six tests in `app::ui` (the press sink and the inverted edges, the five distinct
    faces, the switch's longest word against its cell, a banner declining too little room, the
    widgets really painting — counted in vertices in a headless context, which is the only thing
    that separates "drew a chunky plate" from "allocated space and painted nothing" — and a button
    whose icon is missing still being a button), and by the two rendered frames the defects above
    were found in: `target/shots/shot-dock-edit.png` and `shot-dock-evolve.png`.

    **Partly superseded by finding 55**, which is the pass that followed: a plate now carries no
    caption at all, the switch names only the cell that is on (`segment_label_room` is gone, and so is
    the fixed cell width it was measured against), there are six document tools rather than five, and
    the control the switch's `Fit` shared a row with moved to an atlas glyph. What finding 54 claims
    about *how* a plate behaves — the sink, the inverted edges, the five faces, a disabled control
    ignoring the pointer — is unchanged and still tested.

55. **The window had a look and no composition: the plates were chunky and every one of them
    carried a caption, the editor had a properties column it did not need, and the theme had been
    installed into the one of egui's two style slots nothing was drawing with.**

    Finding 54 dressed the furniture. It did not *remove* any of it, and the report that followed
    was the same report — cluttered, informational, not the graphical feel of a game like Super
    Mario Maker 2. Reading SMM2's editor rather than guessing at it gives four traits that matter
    here, and the two research passes agree on them: tools are **pictures** with their meaning on
    hover; an editing screen carries **almost no text**; the parts live in **one strip**, not in a
    properties pane beside the canvas; and controls read as **physical objects** — rounded, outlined,
    chunky. (The sources are solid on structure and *silent* on measurement: no accessible page gives
    SMM2's panel colours, cell sizes or border thicknesses, and the report says so instead of
    inventing them. So the palette stayed this game's and only the composition was borrowed.)

    Four changes followed, and they are one idea: **what the window says, it says once, and in a
    picture.**

    * **One plate, and no captions.** `ui::chunky_button` (icon *above* name) is now `ui::icon_button`
      (a glyph, period), which is why the second size constant for word-only controls could go: there
      are none. A control's name is on its hover, where a game puts it; the glyph in the plate is the
      game's own art, and for the four controls the world has no picture of — the stroke history, the
      frame-the-level, the details, the restart — it is five new sprites in a new `tools/art/ui.py`
      family, drawn geometrically so the two turn arrows are reflections of one arc rather than two
      hand-typed pictures that can drift a pixel apart. They are the first sprites in the atlas the
      *game* never draws.
    * **The switch names only the cell that is on.** Four labelled cells spent a word on each of the
      three things the window was *not* doing. Now the chosen cell is sized *from its own label* —
      measured in the font it will be drawn in — and its neighbours are bare pictures. That turns a
      guard into a construction: the old code asked whether the longest word fitted in a fixed cell
      and dropped the icon when it did not; there is nothing left to clip.
    * **The editor has no properties column.** The verdict — can the spawn reach the flag — is the one
      reading that has to be up at all times, and it moved into the strip along the bottom; everything
      else (the grid's size, the physics a jump may ask for, the materials, the content hash, the zoom)
      is a **floating window** behind the dock's magnifier. The canvas took the whole width. Half a
      panel of cream saying nothing is the cheapest way to make a game look like an application, and
      that panel was doing it. The run views keep their side panel, and the difference is the point:
      theirs holds the controls a run is *driven* with, so it is busy rather than empty.
    * **The palette says one thing.** `Parts`, then an instruction, then the selected brush named under
      the grid — three lines in the one panel a person looks at the level beside. It is one line now
      (`PARTS · GROUND`), and the instruction is on the cells' hovers, where somebody asking what a
      cell *is* is already looking.

    The second half of the fix was not composition at all, and it is the reason this finding exists
    rather than a paragraph in the review. The first frame rendered with the details window open came
    out **dark**: stock egui window furniture and light type inside a cream window. egui keeps *two*
    styles — one per theme — and picks between them from the system's preference, while
    `Context::set_style` writes only into whichever slot is selected *at that moment*. The theme had
    been installed into the light slot and this machine reports a dark desktop, so everything egui
    styles itself (a window, a button, a slider, a separator, a drag value) drew in egui's defaults,
    while everything that *looked* right — the panels, the dock, the canvas — was right only because it
    paints its own fill through a `Frame`. One `set_theme(ThemePreference::Light)` plus a
    `set_style_of` into **both** slots fixes it, and the regression test asserts the slot nothing on
    screen is using, because `ctx.style()` had been passing the whole time. The general lesson is the
    one finding 54 already states and this pass re-learned the expensive way: **a theme is only
    verified by rendering the widgets it has to reach**, and until this window existed there was not
    one in a frame that egui itself drew.

    Two more defects came out of the same "render it and look" rule, both small and both real. The
    editor's backdrop was a two-tile **transparency checkerboard** over the sky gradient — the picture
    an image tool paints where there is nothing, said about the one surface in the window that is full
    of a person's work — and it is a flat work-surface blue now, with the tile grid (already being
    drawn) as the thing that says where the cells are. And `Atlas::uv` computed a sprite's far edge by
    multiplying a cell index by `1.0 / rows`, which on the new 15-row grid lands **outside** the
    texture: `14 * (1/15) + 1/15 == 1.0000001` in f32. Dividing the index by the grid instead puts the
    last cell's edge at exactly 1.0. The unit-square test from an earlier pass caught it the moment
    the grid changed shape, which is the whole argument for writing that test.

    Pinned by `app::theme::tests::a_theme_switch_cannot_repaint_the_window`,
    `app::atlas::tests::every_glyph_the_window_draws_is_in_the_atlas`,
    `app::ui::tests::the_chosen_cell_has_room_for_the_word_it_shows` (all four modes, measured in the
    font the switch draws with) and `app::atlas::tests::every_shipped_sprite_maps_inside_the_image_at_one_cell`
    (which failed first and is listed here because it is the reason the last two lines of the fix
    exist). The frames are `target/shots/s2-edit.png`, `s2-play.png`, `s3-evolve.png`, `s3-details.png`
    and `s3-narrow.png` — the last at the window's 900-point minimum with the pack's longest level name,
    which is how the dock's width budget is checked rather than assumed.

    **Partly superseded by finding 56**, which is the pass that followed: a plate carries a caption
    again and so does every cell of the switch (the "no captions" rule two bullets up is reversed, and
    `the_chosen_cell_has_room_for_the_word_it_shows` is now
    `every_switch_cell_has_room_for_the_widest_name_in_the_set`), there are nine `ui_*` glyphs rather
    than five, and the two run views' panels are menus whose readings moved into the Details window.
    What this finding establishes about *composition* — one plate size, one vocabulary of plates shared
    by the dock, the palette and the panels, no properties column beside the editor's canvas, and the
    two style slots egui keeps — is unchanged and is what the window is still built on.

56. **Every control in the window was a picture with no name, and the Evolve screen was an
    instrument board where a menu belonged.**

    The report that opened this pass was two lines long: the Edit, Play and Evolve buttons "do not have
    text labels until they are clicked", and on the Evolve screen "there is still a LOT of information
    being thrown at the user". They are one mistake made twice — finding 55 took "few words" and read
    it as "no words", and left a run's panel saying everything it knew at once — and they have the same
    fix, which is to decide what the window *has to* say and put the rest one press away.

    **A caption is not something a control earns by being pressed.** Finding 55 kept the word for the
    switch's *chosen* cell and gave the other three nothing, on the theory that a viewer reads the mode
    they are in and guesses at the rest. The state a person most needs to read the window in is the one
    where they have pressed nothing yet: three bare pictures in a row are a puzzle, and a picture of a
    walker is not a guess at *Evolve*. So every plate in the dock carries its name, all four cells of
    the switch carry theirs, and the words are the price of the bar being usable at first sight. The
    SMM2 reading was not wrong about *count* — the window has no sentences on it — it was wrong about
    zero, and the hover still carries the sentence a word cannot ("step back through the stroke
    history — Ctrl+Z").

    Three consequences fell out of that, each of which had to be got right rather than hoped at:

    * **The switch is one size for all four cells**, taken from the widest name in the set, so the bar
      does not twitch sideways when the mode changes. Sizing each cell from its own label — the rule
      finding 55 introduced — is fine for one labelled cell and a moving target for four; the width is
      measured in the font it is drawn in, and the caption's room is computed by the same padding the
      plate is sized with, which is what the rewritten test pins (a rule and its inset that disagree
      is exactly the sort of drift no assertion about colour sees).
    * **Four more glyphs**, so a run's rows have pictures like a document's do: `ui_pause`, `ui_play`,
      `ui_step` and `ui_medal`, which take the atlas from 228 of 240 cells to 232 — still 16x15, so the
      grid has room, and the *next* family is what widens it.
    * **`ui::menu_row`**, the second half of the vocabulary: a wide, short plate with the glyph at its
      left and its name beside it, which is the shape a column of controls wants, against the dock's
      glyph-over-word. Both are one plate in one socket, drawn by one [`ui::bevel`].

    **The Evolve screen is a menu now.** It was the whole instrument board: on a run with any age on it,
    thirty-odd lines — the search's sigma beside its stall counter, the diversity the archive had
    covered, the x the population kept dying at, a genius note, the curriculum note, the death cluster,
    a best-agent block, a row for every agent in the generation, the recorded champions with their
    scrubber, two sliders and three checkboxes. Every one of those numbers is real and, read at a
    glance, the panel said nothing: the question a viewer watching a population learn to jump is asking
    is *is it getting anywhere*, and the answer was buried under eleven other readings. The panel is now
    **three readings and one word** — which generation of how many, how many of the population reached
    the flag, the best fitness (`menu_readings`) and the trend — plus five rows to press
    (pause/resume, one generation, save champion, restart, details) and two conditions that have to be
    *told* rather than shown: the level has been edited since the run started, and the run is over. The
    replay's panel is the same shape, with the transport in the same rows and the toggles drawn as
    plates that go gold. Both report the same [`panel::Actions`], so a widget's meaning is handled once
    wherever it was drawn — including the new `details`, which opens the window.

    **Nothing was deleted; it moved one press away.** The **Details window** is mode-aware now: the
    level's numbers and its verdict in the editor, and a run's board in Evolve and Replay — the step and
    the stall, the diversity, the max/avg/min of the generation, the death cluster, the champion's own
    record, a row per agent in the field, the recorded champions and their scrubber, the two ladders,
    and the key list. That is the piece that makes a quiet panel a design rather than a loss: the
    numbers a viewer *goes looking* for are all still on screen, behind the magnifier the dock already
    had. The window scrolls, and the field list is capped and scrolls inside it, because a population of
    two hundred ghosts is taller than a display.

    Two duplicated readings went the same way. The dock's banner carries no run's numbers at all any
    more — it was reading out the same generation, flag count and best fitness the panel shows two
    hundred points away — and the evolve status line is "gen 40 · 8 of 8 running · 8x · seed 7" in place
    of a sentence that spelled out the field and timed the animation to a tenth. The replay's line lost
    its ghost roster for the same reason and kept the one fact only it can state: a field that has been
    *hidden* says so, because an empty canvas with no explanation reads as a broken replay. And because
    those two lines are the longest in the window, they are **truncated** rather than allowed to push
    everything after them — the tile under the pointer, the warning — off the edge of the screen.

    Rendered and looked at: `target/shots/v5-evolve.png` (a finished run), `v5-replay.png`,
    `v5-evolve-details.png`, `v5-replay-details.png`, and the 900-point minimum in
    `v5-narrow-edit.png` / `v5-narrow-evolve.png` / `v5-narrow-replay.png`, where a labelled switch and
    a wider dock still fit and the panel is still legible at its narrowest. Pinned by
    `app::ui::tests::every_switch_cell_has_room_for_the_widest_name_in_the_set`,
    `app::panel::tests::the_run_panel_says_three_things` — the *count* is the assertion, because a
    fourth reading added later would pass a test that only checked what the three of them said — and
    `app::panel::tests::an_idle_frame_of_the_menu_asks_for_nothing`, which draws the panel with no
    atlas at all (a tileset older than the glyphs must still be a panel rather than a crash).

57. **Two tiles that make the level do something: brittle ground and a spring pad.**

    The plan deferred springs (line 123: "Defer: flyers, moving platforms, powerups, **springs**"),
    so this is the first **object** the level format has gained since the format existed — and the
    first change to this project whose subject is the world rather than the body. It lands two
    tiles, and they are deliberately not the same kind of thing:

    * a **cracked tile** (`d` soil, `s` sand, `b` brick, `w` timber) is a *material* with a
      behaviour attached — solid exactly as its sound self, drawn with the same autotile family plus
      a crack overlay, and gone `CRUMBLE_TICKS` (**32 ticks**, eight decision blocks) after a body
      **stands** on it. The four variants are not four materials: a brittle beach is a beach, and
      `Tile::base_material` is what keeps the renderer, the shelf's material census and the theme
      from growing a second vocabulary for the same four things.
    * a **spring pad** (`v`) is an *object* in a cell, like a spike: not solid, drawn whole, and it
      throws a **falling** body upward at `spring_v0` = 0.35 — `v0²/(2g)` = **4.90 tiles** of rise and
      **56 ticks** of airtime against a held jump's 2.375 and 40. Only `vy` is written, so the carry
      is `vx × 56`: **5.60 tiles** at run speed, **8.40** at sprint, and a level aims a pad at a
      landing the way it aims a jump.

    **The rule that took the most deciding is the pad's trigger.** It fires on a *descending* body
    (`vy > 0`) that overlaps the launch box, which is one rule rather than two and is what "jumps on
    it" means: a body rising through a pad from below, or walking across one on the floor, passes
    through untouched — so a pad is a target to land on rather than an obstacle in a corridor. The
    alternative (any overlap launches) makes a pad on flat ground impossible to walk past, and it was
    ruled out by the fixture that puts a pad *inside a full jump's arc* and asserts the rising body is
    not thrown while the falling one is. The launch box is the sprite's own extent, not the cell — the
    pad leaves its top four of sixteen rows transparent, and the art and `SPRING_TOP_CLEARANCE` are
    now pinned to each other by a test, which is the cheap version of the lesson §2.12 taught
    expensively about the spike.

    **Found by its own test, and recorded as REVIEW §1.10**: the pad's first version wrote `vy` and
    nothing else, so the *jump cut* fired on the next tick and a pad delivered **0.91 tiles** of its
    advertised 4.90 — worse than a jump. Three of the pad's four rules were already covered and all
    three passed against that. The missing assertion was the one that says a pad is worth using.

    Sensors moved to **v9** without the vector getting wider (98 inputs before and after): what moved
    is what two cells of the patch *say*, a cracked tile at `-0.25` and a pad at `+0.5`. Fitness did
    not move — the reward is unchanged and a pad's throw is deliberately not charged as a jump — and
    the sim did, because a tile that removes itself is new state: `Sim` carries the tiles it has lost
    and the fuses still burning, both folded into `state_hash`, and §"The version trio" has the rest.

    **Verified end to end, on a fixture built for it** (`levels/scratch-trap-tiles.json`: a nine-tile
    cracked span over a pit with three pads at the bottom of it): `reach --level` reports it beatable
    and so does its mirror, and `render --action run` shows both objects in one run — the deck losing
    tiles behind the runner by frame 60, the pad throwing the body five rows up the pit at frame 180
    with `vy` negative, and the flag taken at frame 407 of the level's 1 760-frame cap. In the window,
    `--screenshot out/trap-tiles.png` shows the four cracked palette cells drawing material-plus-crack,
    the pads as plates and coil in their recess, and the app's own verdict — "beatable: flag reachable
    over 3 platform(s)" — on the new level.

    The claim that the four brittle materials are **one behaviour** is an assertion rather than a doc
    comment: `sim::tests::physics::the_four_brittle_materials_are_one_behaviour` runs four levels that
    differ only in which glyph their floor is written in — same actions, same seed — and requires all
    four to agree on the state hash and the observation every frame, to lose the tile on the same tick,
    and to keep the tiles nobody stood on. The observation half is what stops a brittle beach from
    being a second problem to learn; the state half is what stops "one behaviour" from being four
    copies that happen to look alike today.

    **What is not settled**: no champion has been trained on either tile, the ladder has no rung for
    them, and the reachability oracle treats a pad as air (its launch lives in `Sim::step`, not in
    `PlayerState::tick`), so a level whose *only* route is a pad would be reported unbeatable today.
    That, and the two training runs that would turn "learnable" from an argument into a reading, are
    REVIEW §2.13.

58. **The route table was one sweep per *copy* of a level, and that was the whole of the window's
    stutter.** The move graph behind the waypoint sensor ([`sim::reach::goal_routes`]) sweeps every
    stand's canonical moves with real physics, and it costs **23 ms on a 60x15 level** — a number
    nobody had ever printed, because the result is cached and every *cached* read is free. What the
    cache was not is **shared**: `Level` derives `Clone` and the cache was a bare `OnceLock`, which
    `Clone` *copies*. A copy taken before the first ask — and every copy the window and the trainer
    make is taken before the first ask — started empty, and then swept the whole graph itself.

    **The number of copies is what made it a stutter rather than a cost.** One generation of the
    Evolve view is eight lanes, and each lane's episode owns its own `Sim`, which owns its own copy of
    the level: eight sweeps per generation, each landing on the frame that lane first asks for a
    waypoint (**20-41 ms** measured per lane, with a **23 ms** `observation()` inside it). Entering
    Replay is worse — the ghost roster is a hundred runners, so a hundred sweeps. And the trainer paid
    a fresh one per *broken tile*, because a burning brittle tile wrote itself into the grid through
    `set_tile`, which invalidates the cache with the terrain by design: **634 rebuilds and 14 seconds
    of CPU in an 18-second run**, all of it inside episodes.

    The fix is two sentences of principle. **The graph is a pure function of the terrain, so its slot
    is shared between clones** (`Arc<OnceLock<Arc<GoalRoutes>>>`): the document, the run's copy of it,
    each lane's sim and each ghost's sim now ask once and read the one answer, and a mutation replaces
    the `Arc`, so a level whose ground has changed gets a graph for the ground it has now while the
    copies still holding the old ground keep theirs. **And an episode's own crumbling is not an edit
    to the level**: a brittle tile burning out takes the ground away ([`Level::burn_tile`] — the
    physics reads the live grid, so the hole is real) but leaves the route table alone, because the
    flag has not moved and the compass the agent steers by should not swing because a bridge gave way
    under one body's feet. Authoring edits — `set_tile`, `resize`, moving or erasing the flag — still
    drop it, through one named `forget_routes` that every writer of the grid calls.

    **Measured, on the same level and the same population, at pop 200** (frame intervals from the
    window's own loop, p99 and worst frame, before → after):

    * **Evolve**: interval p99 **108 ms → 18.4 ms**, worst frame **180 ms → 22.5 ms**; the frame's own
      `update` p99 **~180 ms → 2.2 ms**, worst **3.0 ms**. A generation's animation now costs
      **0.07 ms** a frame instead of 165 ms.
    * **Replay**: interval p99 → **18.0 ms**, worst **19.2 ms**, `update` worst **1.3 ms**, with a
      hundred bodies on screen.
    * **Training**: **0.299 s → 0.015 s per generation** (median 0.297 → 0.014), **668 → 5 851
      population-episodes a second** on `custom-l1-flat-run` — about **9x**, from not re-sweeping the
      graph per broken tile.

    Held in place by two assertions rather than by the numbers, which is the only form a timing claim
    can take in this suite: `a_route_table_is_shared_by_every_clone_of_one_terrain_and_dropped_by_an_edit`
    compares the graph's address across clones and across an edit, and the brittle-bridge test of
    finding 57 now also asserts that the table a body breaks ground under is the one it started with.

    **What was left on the clock was that first sweep: one job, ~23 ms, per level.** It is now swept
    in parallel, and the shape of the sweep is what decided how. The take-off grid is
    `columns × TAKEOFF_OFFSETS × directions × holds × speeds` — `TAKEOFF_OFFSETS` is **twenty**
    sub-tile positions, so a sixty-tile floor is about **twelve thousand simulated jumps**, and one
    wide platform is most of a level (`l1-flat-run` is a *single* platform, and its sweep is the
    whole cost of its graph). So the parallelism has to reach across the **columns**, not across
    platforms: `edges_from_all` flattens every `(platform, column)` job into one list, sweeps it on a
    scoped pool of `cores - 1` threads — the same one-core-for-the-window trade the trainer's pool
    makes — and assembles the pieces by chunk index. That last clause is the whole safety argument,
    and it is asserted rather than asserted-in-a-comment:
    `a_parallel_sweep_answers_exactly_what_a_sequential_one_does` compares the threaded sweep with the
    sequential one, edge for edge, on both job shapes the crate asks for, and it fails if the
    assembly stops respecting the order (checked, by scrambling it: a scheduler is the last thing a
    determinism guarantee should quietly rest on).

    **Measured end to end, on the same levels, queued against the sequential sweep:**

    * the **route-graph sweep**: 24.7 / 28.0 / 31.3 / 27.3 ms → **7.6 / 8.5 / 8.1 / 6.7 ms** for
      `custom-l1-flat-run`, `l1-flat-run`, `l6-marathon` and `l9-wait-and-climb` — **~3.5-4.2x**;
    * the **editor's oracle** (the same sweeps under a different question): **22.8 → 7.1 ms**;
    * the **first generation**: 40.5 → **26.7 ms**, and the remainder is the generation's own
      evaluation rather than anything the graph does;
    * `train-cli reach` as a *process*, startup included: **27 → 9 ms**;
    * training, which pays the sweep once: 0.015 → **0.011 s/generation** (**5 851 → 7 668**
      population-episodes a second).

    **One plausible fix was killed by its own measurement**, and it is worth writing down because the
    guess was good and the numbers were not. The same stall was attributed to `Sim` owning a *copy* of
    the level, and the obvious repair was a shared `Arc<Level>`. Measured instead: `Level::clone` is
    **0.04 µs**, `Sim::new` (walker-less level, warm graph) **0.2-0.3 µs**, `Replay::new` over **200**
    runners **0.46 ms**, `Lanes::new` over 8 lanes **0.04 ms**. The roster was never the cost, and the
    108 ms "replay entrance" in finding 58 was never a roster build either: it is `prime_scripted_run`
    waiting for three generations, which is `--replay` doing exactly what it says. So no `Arc<Level>`
    was added — a copy of a small level is cheaper than the atomic it would be replaced with.

    **What is still on the clock**, honestly: the sweep is *faster*, not *smaller*. It is still
    `columns × twenty offsets × directions × holds × speeds` physics simulations, and a level twice as
    wide costs twice as much; the structural saving would have to come from the move set itself, which
    is the oracle's semantics rather than its plumbing. And the editor's oracle still runs on the
    frame that asks for it — 7 ms of a 16.7 ms budget is not nothing. The next step there is not a
    faster sweep but moving the verdict **off the frame**, the way a run's generations already are:
    hash-keyed, computed on a worker, adopted when it lands and dropped if the document has moved on.
    That is REVIEW §2.14.

59. **A level's two expensive answers were recomputed by every process that opened it, and they are
    pure functions of the level.** The route table ([`sim::reach::goal_routes`]) and the oracle's
    verdict ([`sim::reach::report`]) both sweep every canonical move on the terrain — **10.1** and
    **8.6 ms** on `l9-wait-and-climb` after finding 58's parallel sweep — and neither is a function of
    anything but the grid, the spawn and the flag. Yet every process that opened the level paid them
    again: the window's own open, every `train-cli` command, every test that loads a shipped course.
    Both now live beside the level, in `<level>.json.routes`, and are adopted at load.

    **The key is the content, and it carries two versions.** A sidecar is used only when the level's
    own `content_hash_string` matches — the grid, the spawn, the flag, which is everything either
    answer is a function of — *and* when both `sim::SIM_VERSION` and the new
    `sim::reach::MOVE_SET_VERSION` match. The second constant exists for this: `SIM_VERSION`'s
    contract is physics, tile semantics and episode termination, and the **oracle** changes on its
    own as well (the take-off grid went from five sub-tile offsets to twenty; a climb's script or a
    new direction is the same kind of change), without touching a physics constant. An answer is only
    worth keeping while the same oracle would give it again, so the move set is stamped beside the
    sim version. Staleness is then not something to remember: edit a level, save it, and the hash has
    moved, so the next open recomputes and rewrites — there is no cache to invalidate by hand.

    **Measured, a fresh process each time, on `l9-wait-and-climb`** (release, the sidecar absent →
    present):

    | | cold (no sidecar) | warm (sidecar) |
    |---|---|---|
    | open the level, hash it | 35 µs | 53 µs |
    | route table (`Level::routes`) | **10.1 ms** | **36 ns** |
    | oracle verdict (`reach::report`) | **8.6 ms** | **486 ns** |
    | `train-cli reach`, whole process | 24 ms | **2 ms** |
    | `train-cli train` pop 8, 1 generation, whole process | 21 ms | **5-6 ms** |
    | that run's generation 0 (which pays the route table) | 10-12 ms | **3 ms** |

    Eighteen microseconds of sidecar read is the whole price; the two sweeps it replaces are 18.7 ms.

    **Four tests hold it, and each was checked to bite by breaking the mechanism it covers** — a cache
    is exactly the kind of thing that passes its own tests while serving nothing. Adoption is made
    *visible* by **poisoning** the file: the graph's hop counts are rewritten to `u32::MAX` and the
    verdict's diagnosis to a sentence no oracle says, so a reopened level answering with them is
    direct evidence that the answer came off the disk rather than out of a sweep (a timing would be
    evidence about the machine). Invalidation is checked input by input — a tile, the spawn, the flag,
    a walker, the name — because a key can only fail by *matching* for a level it does not describe. A
    file that is not JSON, or empty, or caught mid-write, or of another format, version, build, move
    set or level is a miss and never an error, and a file holding only **one** of the two answers is
    *early* rather than broken (the two are produced by different asks, so whichever comes first is
    written on its own and the other half lands on the next write). And a document that has been
    edited neither reads nor writes the file's cache — the interesting case being the editor, where a
    level that adopted its verdict on frame one must not keep serving it after the user has moved the
    flag.

    **Two things the work turned up that are worth more than the cache itself.** The sidecar's name
    must not end in `.json`: every tool in this workspace finds levels by that extension, so the first
    version (`l1-flat-run.routes.json`) was offered up to the shelf as a level — `l1-flat-run.routes
    did not load`, which is the shelf's own test catching it. It is `<level>.json.routes` now,
    invisible to all of them by construction rather than because each of them remembered to skip it.
    And the verdict is kept for the oracle's *own* question only (`ReachOptions::default`): the
    callers that ask with an ability switched off are tests checking that a level *needs* it, and an
    entry per ability set would rewrite the file every time and hand the editor a miss for the
    question it actually asks.

    **What this does not close.** A cache answers for a **file**, so it does nothing for the editor's
    drag: every paint stroke makes the document a different level, which is exactly why it must give
    up its claim to the file's answers, and the verdict for that stroke is still swept on the frame
    (REVIEW §2.14, now open only for the edit). What is closed is the **open** — the window's first
    frame on a level, the trainer's first generation, `train-cli reach` — and it is closed for good,
    because that cost was never about the machine being fast; it was the same answer being computed
    again by everyone who asked.

60. **The reachability oracle did not model spring pads, so a level whose only way across a gap was a
    pad was reported *unbeatable* while the population crossed it.** Reported from play rather than
    found by the suite: a level built by hand — a pad on the lip of a wide gap — showed the editor's
    `NOT beatable: no take-off from platform 1 clears the 6-tile gap: a sprinting jump covers 6.00
    tiles and must land past it`, and evolution solved it in a handful of generations. The oracle's
    own contract is that it drives the **real** stepper "so it can never disagree with the sim about
    how far a jump goes"; what it drove was `PlayerState::tick`, which is one layer below the rule
    that owns a pad. A pad lives in the *episode loop* (`Sim::step`: a body whose box overlaps
    `spring_launch_box` with `vy > 0` is thrown at `spring_v0`, keeping its `vx`), so every canonical
    trajectory the sweep built was a trajectory through a game with no pads in it.

    **The fix is a shared call, not a second model.** The rule is now `physics::spring_launch` — the
    box, the descending-only condition and the `bounce` in one place, beside the height and the carry
    it produces — and both callers run it at the same point in their own tick: `Sim::step`, and the
    oracle's `oracle_tick` inside every canonical trajectory. `ReachOptions` gained a third ability
    (`spring`, on by default) so "this level needs a pad" is a claim a test can check, and
    `MOVE_SET_VERSION` went **1 → 2**: a route-cache sidecar holds a *verdict*, and a stale
    "NOT beatable" about terrain a population can cross is exactly what must not be reused
    (finding 59 is the key that makes that automatic).

    **The boundary is the finding**, so it was swept rather than asserted. A fixture with a pad on the
    lip of a pit of *n* columns, one goal on the far side, pop-free and deterministic:

    | pit (columns) | oracle, pads on | oracle, `spring: false` | a sprint plus one jump near the lip, through the real episode loop |
    |---|---|---|---|
    | 5 | beatable | **beatable** — a sprint jump covers 6.00 tiles | crosses (and is thrown on the way) |
    | 6 | beatable | **beatable** | crosses, thrown |
    | **7** | **beatable** | **unreachable** | **crosses: thrown from inside the pad's column at `-spring_v0`** |
    | 8 | beatable | unreachable | crosses, thrown |
    | 9 | beatable | unreachable | *this* script (one gear, one jump) does not find it; the oracle's 20-offset sweep does |

    Seven columns is the boundary because a sprinting jump covers **6.00** tiles and the body is **0.60**
    wide: six columns are a jump and the seventh is not. So the fixture in the oracle's own tests uses
    seven, and the pad's part in the crossing is unambiguous — a body that comes down on the pad keeps
    the `vx` it arrived with and is carried all **56 ticks** of the throw at it, **5.60 tiles at a run
    and 8.40 at a sprint**, arriving from **4.90 tiles up** rather than a jump's 2.37.

    **Held by three tests, one of which is the promise itself.**
    `a_pad_at_the_lip_is_what_crosses_a_gap_no_jump_can` (the same terrain answers both ways,
    depending on whether the oracle may use a pad), `the_sim_crosses_the_gap_the_oracle_credits_to_the_pad`
    (the credited crossing is driven through `Sim::step`, and the run that wins is asserted to have
    been *thrown* — so the oracle cannot be crediting a pad for a jump's work), and
    `walking_off_a_pad_at_a_ledge_is_never_a_launch` (a body walking over a pad is not thrown, in the
    sim and in the oracle: the rule is about *arriving* on a pad, not touching it). Through the
    interface the report came from, the owner's own level now reads
    `train-cli reach --level levels/custom-l1-flat-run.json` → **OK**, platforms 3 reachable 3, and
    every other level in `levels/` answers exactly as it did before, the genuinely unreachable scratch
    tower included.

    **The lesson worth carrying forward.** The oracle is an *equivalence claim*: model the game's rules
    or the answers are wrong in the expensive direction. Sprint is arithmetic the report prints, a wall
    climb is a script, and a pad turned out to be neither — it is a **tick rule**, and a rule that lives
    between two ticks is invisible to an oracle built on the body's stepper. The next mechanic of this
    kind (a dash, a carry, a bounce) has to be asked the same question first: does it happen inside
    `PlayerState::tick`, or in the loop around it?

61. **Every genome of a generation was judged on its own patrol phase, so the tournament compared
    worlds — and the fix is a seed formula, not a reward.** `suite_score` derived each episode from
    `episode_seed(master_seed, generation, genome_id, trial)`, and on a walker level the patrol phase
    comes out of that seed. Genome 7 and genome 8 of one generation therefore faced different worlds,
    and the ranking between them was partly a ranking of *draws*: the workarounds the pack already
    carried — three trials, and the `rank_pessimism` knob — exist to average out noise the seed
    formula injects, which is a way of pricing a defect rather than removing it.

    **The defect was measurable, and the old test was measuring it.** On a walker fixture at pop 16,
    generation 0: without pairing, 14 of 16 genomes scored their three trials differently and there
    were **24** mean-versus-floor reorder pairs; with pairing, **0** of 16 differed and there were
    **0** reorder pairs. Every genome's three trial traces are distinct in both arms, so this is not a
    degenerate episode — what the unpaired policy hands out is a different *world* per genome. So the
    assertion the pessimism test used to make at generation 0 ("some pair of genomes disagrees about
    the average and the worst") was satisfied by the lottery, not by the phase reaching the score.
    Five generations later the population has met the walker and the real effect appears: 16 of 16
    genomes differ, mean spread 688 fitness units, 8 reorder pairs.

    **The fix.** `sim::rng::common_episode_seed` is `episode_seed` with the genome id fixed at
    `NO_GENOME` — one frozen mixer for both policies — and `GaConfig::common_episodes` (on by default)
    has every genome of a generation derive `(master_seed, generation, trial)`. The generation is
    still mixed in, so the world re-rolls every generation and no phase can be memorised by rote, and
    nothing about the reward, the physics or the observation moves: no version bump, no champion
    invalidated, and a champion still records trial 0 of its own level, so `replay` reproduces
    exactly what it did before. `episode_seed_for` is the one place the decision is made, and the
    trainer, the suite scorer and `train-cli score` all ask it rather than each building a seed.

    **Paired episodes make the extra trials spendable, which is the part that pays.** `race_episodes`
    (also on by default) judges everyone on trial 0, then only the better half on trial 1, then only
    the better quarter on trial 2 — successive halving on the same pessimistic score selection ranks
    by, halved by the same keep-half rule, and unsound without pairing because an apparent leader
    might simply have drawn an easy phase. Cost, at pop 200 on `l3-gauntlet`, 600 generations, two
    repetitions each, `--stop-on-win` off:

    | arm | episodes per genome | run 1 | run 2 |
    |---|---|---|---|
    | paired + raced (default) | 1.75 | 11.21 s | 11.17 s |
    | per-genome, flat (old) | 3.00 | 15.73 s | 15.74 s |

    That is 29 % off the clock — the 42 % off the *episode* work diluted by the third of a generation
    that is breeding and bookkeeping — for the same generation budget. The episode counts are pinned
    by tests that read what the racing loop spent rather than the formula: 28 against a flat 48 at pop
    16 on a walker fixture, and 22 against 32 on a suite of one walker level and one plain one.

    **What it did to the pack, twelve seeds at pop 200, `--stop-on-win`, and the honest reading of
    it.** `l3-gauntlet`: 12 of 12 either way (median 13.0 paired against 16.0 unpaired, p = 0.77).
    `l6-marathon`: 12 of 12 in all three arms at the same median 10.5. `l9-wait-and-climb`, the
    pack's slowest level: **7 of 12 paired against 10 of 12 unpaired** (median 116.0 against 97.5),
    with the raced arm back at 10 of 12. Four seeds solve without pairing and not with it (2, 6, 8,
    12) against one the other way (11), so the exact sign test over those five discordant seeds is
    **p ≈ 0.13**: suggestive, not established, and it is the arm that argues against the default. The
    reading this file commits to is that pairing is a **wash where the level is already easy for the
    population and a possible cost on the one hard chain level**, in exchange for a within-generation
    comparison that is about policies rather than worlds — and that racing, which pairing is the
    precondition of, buys back the generation budget that pays for it.

    **What this does not close.** Pairing fixes the noise *inside* a generation and nothing else:
    `best_ever`, the stagnation counter and the elites still carry a score taken on another
    generation's world. A **culled** genome's `worst_trial` is a floor over fewer episodes than a
    survivor's, so the two are not comparable after a cull (nothing selects on it then, which is why
    it is a documented consequence rather than a bug) — and that is why a bench arm now records the
    *average* episodes per genome beside the trial count. And the ES shares `evaluate_population`, so
    it inherits pairing without its own arms having been re-taken.

62. **The held-arm sensor slot was built, measured, and reverted: it cost the route clone's warm
    start and bought nothing measurable.** The gap was real: `PlayerState::jump_held` is part of the
    state hash, the jump is edge-triggered (`want_jump && !jump_held`), so a policy holding jump
    through a descent cannot press on the tick it lands — and no combination of `vy`, `can_jump` and
    the patch says whether the button is down. One slot was added at index 100 (`sensors` v11, 101
    inputs), the golden observation hash re-pinned, the layout table updated, and then it was measured
    against three arms.

    | what was measured | v10 (no slot) | v11 (slot) |
    |---|---|---|
    | `gauntlet`, 12 seeds, hand-paired (the tool refuses across a version trio) | median 13.0, mean 20.6 | median 12.0, mean 12.8 |
    | the same seeds, discordant | 7 seeds faster | 5 seeds faster (p ≈ 0.77) |
    | `l4-rhythm` pop 64, seeds 1-6, inside 60 generations | 4 of 6 (at 4, 4, 28, 32) | 4 of 6 (at 8, 7, 4, 9) |
    | warm start `l2-first-gap`, six paired seeds, total generations (cold 32 → 35) | **23** | **57** |
    | warm start `l5-tightrope`, same six seeds (cold 171 → 152) | **56** | **143** |

    The last two rows decided it. The route clone is the one path in this project whose whole value is
    being a better starting point than noise, and at 101 inputs the same fixed-budget fit is a worse
    warm start on the level a warm start exists for: `l5-tightrope` solved six of six seeds with the
    clone at v10 against five of six at v11, 56 generations against 143, and the imitation canary
    (`a_warm_quarter_learns_sooner_than_random`) fails outright on `l2-first-gap` (warm 41 against
    cold 18) where it passes with the slot removed and *everything else in this pass in place* — which
    is how the cause was isolated to the slot rather than to pairing or racing. So the slot would have
    been landed against a measured cost on a headline feature with no measured benefit, for a version
    bump and the retrain of every champion.

    **The finding worth keeping is not "the slot is bad".** It is that a sensor slot has to be measured
    against the *imitation* path as well as against the search, and that the fit is sensitive to the
    input width: a 4000-epoch supervised fit over one more column is a slightly worse policy, and
    "slightly worse clone" is a different failure from "slightly worse search". Two ways to make it
    land: re-fit the clone at the new width (the fit's budget, not its inputs, looks like the binding
    constraint), or make the slot carry *move* information — where the take-off is, how fast to be
    going — instead of a redundancy of the body's own state (REVIEW §4 item 4, which is now the more
    attractive of the two). The slot is not in the tree; the measurement is.

63. **Two canaries pinned a single draw where the claim was a rate, and a sensor change is a re-roll of
    every random population.** Both fired in the same pass and for the same reason — the genome layout
    moved, so every random initial population changed. The first required `l4-rhythm` at pop 64 on
    **seed 2** within 60 generations; measured, four of six seeds solve inside 60 (at generations 4,
    4, 28 and 32) and the other two stall past 300, so a single seed is a coin flip and the pinned one
    happened to be a winner on the build it was written against. It now sweeps four seeds and requires
    two, which is the claim that was actually measured. The second asserted that no ghost lane shares
    the star's seed — true of a per-genome episode policy and false of a paired one by construction —
    and now asks the trainer what the policy is and checks the lanes against it either way.

    Neither is a weakened assertion, and the distinction is the point: the first was measuring one
    64-net draw and now measures a rate, the second was pinned to the policy it was written under and
    is now pinned to the policy in force. A third test changed for the opposite reason: the walker-floor
    test used to assert a mean-versus-floor disagreement at generation 0, which (finding 61) was the
    world lottery, and it now sweeps the first eight generations, asserting the invariants on each and
    the premise — that the phase reaches *somebody's* score — on the population it finds.

# Enemies and objects: five mechanics, and what makes each one a decision

The pass added a chaser, a flyer, a turret with projectiles, a one-way platform tile and a moving
platform entity. The design rule for all five was the same: **fit an existing rule rather than add a
mode**, because every new mode is a new thing the population's observation has to distinguish, and
the observation is the expensive surface.

| thing | what it adds | rule it reuses |
|---|---|---|
| **one-way platform** (`=`, `Tile::OneWay`) | a route that goes up through a floor | ground from above, air from below: the first tile whose `is_solid` and `is_opaque` answers differ |
| **chaser** (`chaser` entity) | a threat that closes distance | `Walker` with `WalkerMode::Chase`: same body, same stomp, same contact |
| **flyer** (`flyer` entity, `span` in tiles) | a threat that holds a height | gravity-free patrol between authored ends, plus the walker's stomp rule one row up |
| **turret** (`turret` entity, `dir`) | a row that is dangerous on a clock | a stationary enemy that fires `Shot`s along its own row, on `turret_period` ticks, bounded by walls and `shot_range` |
| **moving platform** (`platform` entity, `tiles`/`span`/`axis`) | a route that is not terrain | the one-way rule stated as a *box* rather than a tile, with the rider carried, a **crossing** landing test and a travel clamped to the clear part of its path |

The three numbers that were derived rather than tuned:

* **The chaser stops at a ledge instead of falling off it.** It is the difference between an enemy and
a level's own auto-solver: a chaser that walks into the first pit kills itself the moment a body
crosses a gap, which makes every level with a chaser in it a level that plays itself. Stopping also
makes baiting one to an edge a legitimate answer, which is the kind of thing a difficulty curve wants
to *teach* rather than forbid. A patrol still turns at a ledge, so nothing about walker levels changed.
* **A mover's position is a function of the frame count, not accumulated state.** `Mover::rect_at` is
a triangle wave over `home`, `span` and `mover_speed`, so two runs of one seed agree without hashing a
phase, a platform cannot drift from float accumulation, and any frame can be asked where the shelf was
without replaying to it. The only state is the rider (`Sim::riding`).
* **The shell doesn't have to walk.** `is_solid` is the surface question and `is_opaque` the wall and
ceiling question, and the one-way tile is the only thing in the pack that answers them differently.
That is exactly one match arm apart in each of the four places that ask — the player's x resolution,
the ceiling test, the walkers' wall probe, the sensors' patch — which is why the tile cost a fraction
of what a fifth mode would have.

**The oracle does not model the mover, and that is a decision.** A route is a statement about the
terrain; a mover is not terrain, because the sweep would have to know which frame it will be at, which
is a fact about the episode rather than about the level. So a level with a mover is swept as if the
mover were not there. That makes the rule for authors a single sentence — *a level that needs a mover
reads as unbeatable, so a mover is a shortcut or a decoration, never the only route* — and it is why
the demo level's slider crosses a five-tile pit that a sprint jump also clears. The alternative,
refusing to answer at all, would make every level with a lift unverifiable.

What is verified, all of it by `cargo test --workspace`: ten new integration tests in
`crates/sim/tests/entities.rs` (the shelf stops a fall and passes a rising body; the chaser closes
distance and stops at the edge; the flyer turns at both ends of its span; the turret fires the way it
faces and a shot ends the episode; a mover carries its rider; every new entity round-trips through the
writer with its fields; the sensors see a flyer and read the shelf as its own value; the oracle stands
on a shelf), the editor's own tests (each new brush places, is pickable, is erasable, and refuses
what the loader would refuse; a pick adopts a turret's facing and a platform's shape), the atlas test
that every palette brush has an icon, and the pack's round trip with the new level in it.

`train-cli reach --level levels/l10-the-hunt.json` reports it beatable (`platforms 4, reachable 4`)
and `train-cli render` shows all five on one screen — `F` the flyer over the shelf, `h` the chaser on
the middle ledge, `T` the turret behind it, `MMM` the slider and `=` the shelf.

What **is** measured, on the demo level: at pop 40 and 60 generations, `l10-the-hunt` was solved on
**4 of 4 seeds** on the first build (first wins at generations 8, 12, 25, 26; best runs 6.3-6.5 s /
374-387 frames, `replay` bit for bit, `stomps 2, jumps 5, checkpoints 3`), and **2 of 2 seeds** after
the mover fixes and the wider flyer beat (first wins at 33 and 7, bests 6.15 s and 6.87 s / 369-412
frames) — the level is still won across the one-way shelf, which is the claim the oracle cannot make. The crossing is the
shelf: the first pit is twelve tiles wide, so no jump clears it and a win means the policy went up
through a one-way platform and along it. That is one careful reading, not a difficulty study — pop 40
is small and the pack's slow level takes more than twice this many generations at pop 200 — but it is
the claim the oracle cannot make and the reason the level is in the pack.

What is **not** measured, and should be read as open:

* **Whether the enemies teach anything.** A policy won while stomping two of them, which is evidence
  that they are *survivable*, not that they are *curriculum*: no arm has compared a level with and
  without them, so the honest statement is that the five mechanics are correct as physics, visible in
  the sensors, placeable in the editor, and won through by one small population.
* **The flyer's span and the turret's period are authored numbers with no measurement behind them.**
Four tiles and 120 ticks are chosen to be legible at `run_speed` — a flyer crosses its beat in about
the time a run jump is in the air — not because a sweep said so.
* **Two document operations had to learn about the new entities, and both are tested.** `mirror`
  reflects every kind and flips the two things that face: a turret's firing direction and a flyer's
  two ends (a slider still travels right); `resize` drops what the new grid cannot hold, which it did
  not at first — an entity left outside the grid makes `validate` refuse the document, so a shrink
  turned "a level that lost a flyer" into "a level that cannot be saved".
* **A mover is the one mechanic that can make a level's oracle verdict wrong in the *safe* direction**
(it says unreachable for a level whose only route is a shelf that moves), and there is no test that
says so beyond the demo's own design note.

# The elements: three tiles, and a fluid that is drawn like the ground

The pass added water (`~`), lava (`L`) and vines (`V`), and the numbers that make them
mechanics rather than decoration are in the README's physics table. What this section is
for is the *drawing*, because fluids are the one family in the atlas that cannot be drawn
one cell at a time and the first two attempts at them were wrong in instructive ways.

A solid tile is a **mask**: `ground_mask` asks the four neighbours, the mask picks one of
sixteen crown/body sprites, and the art is the art of that tile. A fluid cannot work that
way, because a fluid has no silhouette — the basin it is cut into is its silhouette — and
because two stacked fluid cells are supposed to be *one body of liquid*. So a fluid cell is
built out of four pieces instead, and its neighbours decide which pieces it gets:

| piece | chosen by | shape of the answer |
|---|---|---|
| body | the cell's position modulo four into one picture four tiles square | a texture, continuous across every cut |
| surface | nothing above it is the same fluid | two frames, phased by column |
| bed | nothing below it is the same fluid | an overlay over the body cell |
| wall | the side is neither the same fluid nor solid | an overlay, mirrored for the right |

**Three things that looked like answers and were not**, each caught by looking at a pool ten
cells deep rather than by reasoning:

* **A body with a depth gradient in the cell.** Every cell drew light at its top and dark at
  its bottom, so a two-deep pool drew the gradient twice and the boundary between the cells
  was a band. The fix was to move all of that structure out of the body and into the cells a
  pool *ends* in, which occur once per column: the surface and the bed.
* **A per-cell variant, the ground's `_b`/`_c` trick.** It removes the repeat and replaces it
  with a seam: two different textures meeting at a cell boundary draw a line there, and a
  pool has a boundary at every tile. A four-cell pool hid it; a ten-cell column showed a stack
  of shelves. The answer is the one every tile game uses for a large surface — one picture,
  cut into cells and indexed by position — so the picture is continuous *at* the cuts and the
  repeat is four tiles instead of one.
* **A per-cell tint for depth.** The same failure as the variant, one step subtler: a uniform
  4.5% darkening per cell below the surface is a gradient at a distance and a visible step at
  every boundary up close. Depth is the bed's job; there is no value per cell anywhere in a
  fluid, because a value per cell *is* the grid.

The renderer's side is `view::fluid_body_sprite` (the block index) and
`view::draw_fluid_edges` (the bed and the two walls), which is a few lines each precisely
because the art is cut that way. The one number the two languages share is
`FLUID_BLOCK = 4`, and `view::tests::fluid_bodies_tile_as_one_four_by_four_picture` pins it:
all sixteen cells per fluid must be in the shipped atlas, the index must wrap at four in both
axes, and the thirty-two cells must be thirty-two different pictures rather than one pasted
twice over. That last assertion is the one that would have caught the first attempt.

What is **not** measured: whether a pool now *reads* as liquid is a judgement made by looking
at screenshots of a four-deep pool, a ten-deep column and the shipped level, not a number.
The surface is still one cell of every pool, so a very tall pool has exactly one waterline;
the walls are only drawn against open air, so a basin's own walls are still the terrain's.

# The M3 problem

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
   The pack is eight levels wide and every one of them falls to evolution: from L1
   (a random population wins it) to L3's median of 20 generations, with L7 and L8 the
   two levels a population can only win by *finding a move* — a wall climb and a
   sprint jump (findings 43–44). Getting there needed the measurements in
   findings 15–18, and the useful form of the goal turned out to be a *discoverable*
   ladder: every atom has to be learnable on its own before the next one arrives,
   which is exactly what the trap in finding 15 forbids. What did **not** work:
   length (a 124-tile marathon of hard atoms was never solved), repeated obstacle
   types (the population generalises "jump when the gap is in front" within three
   generations), or walkers (they add texture and almost no difficulty). What was
   true when this list was written and is not any more is *walls*: bunny-hopping
   cleared them from generation 0, and the fix was not a level but the hop — a jump
   now needs a fresh press (finding 43), and L7 is built out of the move that
   replaced it.
3. **Diversity before fitness.** With pop 200 and no speciation the population can
   collapse onto "hop right" within a few generations — which is exactly the failure
   the champion-only view would hide. Best-4 + random-4 (Tier 1.5) makes it visible,
   and the death strip turns "where is it stuck" into a picture rather than a
   guess.
4. **Say when it has stopped moving.** — *done, see finding 19.* A stall line that
   reports how long since the population last found anything better, and which tile
   column its agents keep dying on. It is the difference between watching a level
   that is being learned and watching one that is not, and on the shipped pack it
   fires for real: on **L7 seed 12** — the slowest seed of the slowest-to-learn level
   — it appears at generation 25 (`no new best in 25 generations (best 373.7 set at
   gen 0)`) and keeps counting to **89**, with the population's best frozen at that
   generation-0 wall the whole time, until an agent reaches the flag at generation 91.
   That is a watch run where the line is the story.

What is *not* answered by any of that is the gate itself. Tier 1 is complete and
exercised, and the instrument is now calibrated — a pack whose slowest levels are
1.3 and 1.0 minutes at the default speed and 10.7 and 8.0 minutes at 1×, with the
stall line above living through most of the longer one — but "is watching this fun
for 10 minutes" is still a human judgement, and the honest status is: the instrument
exists and is pointed at a level hard enough to need it, and the measurement has not
been taken.
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
   the stall: on L7 seed 12 the record holds **one** entry — generation 0 — for the
   whole 89-generation stall item 4 measures, and the two that follow it are the
   generations the wall climb finally broke through. One improvement, then the win.

7. **Move the stuck population somewhere it can learn.** — *done, see finding 22.*
   `--curriculum <easier level>` makes the first stall checkpoint the population, train
   it there for 40 generations and hand it back, and `watch --resume` restarts from that
   checkpoint. On the seed that never solved L5 in any number of generations, this
   solves it (generation 163 via L3, 418 via L4). This closes the plan's stall chain —
   detector, death-spot highlight, resume, honest charts — with the measurement the
   plan asked for and the numbers this file can be held to.

8. **Hand the thing to a human, at a keyboard.** — *partly done, see finding 23.*
   `play` is M1's exit tool and the gate's final instrument, and it had never been
driven by a real terminal. Doing that found and fixed the reason a first playtest
looked broken (one frame per press), and it turned the exit criterion into two
measurements instead of an assumption: L1 and L2 are beatable by holding right and
jumping when something appears, L3 is not measured, and the whole check now runs in
CI (`crates/render/tests/playability.rs`) rather than in someone's memory.

# The elements, part two: what a route needs to know, and what a frame can pay for

The element pass above was the tiles and their drawing. This one is what the **search**
does with them, and it started from two reports that turned out to be two sides of one
question: the oracle marked long stretches of water unreachable, and the agents ignored
vines.

## The oracle's move set, at `MOVE_SET_VERSION` 6

The oracle knew how to jump, fall, wall-jump and ride a spring. It did not know that a
body can *swim* a pool and climb out, or that a vine is a route. So a lake between two
banks — the most ordinary thing water is for — had no edge across it, and the goal on the
far side read as unreachable however obviously a swimmer could cross. The fix is two
moves of the graph rather than a special case in the verdict:

* `RouteMove::Swim { dir, dive }` — a stroke from a surface cell in one direction, with the
  dive variant for the pool that has to be crossed under a lip;
* `RouteMove::Vine { exit }` — grab, climb the run to its top, and leave toward `exit`.

Both are ordinary graph edges: the sweep simulates them with the oracle's own simulator
(`simulate_swim`, `simulate_vine_climb`), `press_lands` asks the same "does the script,
run from here, reach the stand the graph promised" question it asks of a jump, and the
route replay executes them at the policy's cadence (`vine_action` holds the button for the
climb and steers for the exit; `swim_action` presses for the stroke). `MOVE_SET_VERSION`
is 6 because an answer is only worth caching while the same oracle would give it again, so
every cached verdict from a five-move graph was thrown away rather than trusted.

**Measured on purpose-built levels**, oracle verdict first and then a real run (pop 200,
seed 1, `train-cli train`, first-win generation):

| level | shape | oracle | first win |
|---|---|---|---|
| `lake-42` | 42-tile lake, banks level with the surface | reachable | — |
| `lake-lip` | 22-tile pool, banks one tile above the water | reachable | gen 0 |
| `lip0`-`lip3` | same bank, lifted 0-3 tiles above the water | reachable | — |
| `vine-gap` | 8-tile pit with a vine hanging into it, far ledge two tiles up | reachable | gen 11 (149 frames, 3 jumps) |
| `vine-route` | a vine shaft to a ledge above the spawn | reachable | gen 1 |
| `l11-the-elements` | the shipped level: lava, a pool, a three-tile wall and a seven-tile vine shaft | reachable, 8 of 9 stands | gen 4 (458 frames) |

So the two claims that started this are both false of the current graph: a long pool is an
edge, and a vine is a route the population finds. What is *not* measured is the human
half — whether watching an agent climb reads as climbing — because that is a judgement
about the animation, not a number.

## The stall's detour, off the packet path

The one freeze the window could not hide was its own: a stalled run's practice detour —
forty generations on an easier level, plus the sweep that derives that level, plus two
file writes — ran between two generations. **1.73 s on `scratch-tower`** against a 20-50 ms
median generation, and a generation's animation is 0.6-5 s, so the viewer saw a still
frame for the difference. One generation of lookahead could not cover forty.

It runs on its own thread now (`lab::Detour`): the detour takes a clone of the trainer,
the worker keeps producing generations from the run's own population, and the practised
population is adopted the generation after the thread lands (`Trainer::adopt`, the same
entry a curriculum step always returned through). At a cap the run waits for a detour in
flight — the note and the artifacts are the run's last word, and nothing is animating
behind it.

Measured, `LAB_PROFILE=1` on `scratch-tower` at pop 200:

| reading | before | after |
|---|---|---|
| detour cost | 1.73-2.44 s | 2.44 s, on another thread |
| slowest packet in 120 generations | ~2.4 s | **69 ms** |

And in the window itself, `EVO_FRAME_LOG=1 evo-app --evolve --no-splash` now prints every
frame over two vsyncs; 40 s on `scratch-tower` and 25 s on `l11-the-elements`, stall and
detour included, produced **zero** such frames. The instrument is the point as much as the
fix: "the window freezes" could not be attributed from a log that did not exist.

Still on the UI thread: the warm start (`warm_seed_from_route`) runs inside
`Lab::start`, which is the window's own call — 0.3 s measured on a cold route cache
(`l11`, pop 64), so it is a hitch rather than a freeze, and it is the next thing to move
if a bigger level makes it one.

# The polish pass: ink you can read, and two kinds of line

The window's look was already a designed one — cream chrome, dark screens, bevels, a gold lip —
so this pass was not a redesign. It was the measurement the palette had never had, and the three
defects that measurement found. All ratios below are WCAG 2.x relative-luminance contrast.

## Type: 4.5:1 on every surface it is drawn on

The chrome has four depths (panel, card, card-hover, sunken well) and the palette had one ink
scale, chosen by eye. Measured, the small print of the window was partly unreadable:

| ink | on a panel, before | on the sunken status bar, before | after |
|---|---|---|---|
| `TEXT_DIM` | 4.49 | 3.58 | **5.82 / 4.64** |
| `TEXT_FAINT` | 2.43 | 1.93 | **5.07 / 4.04** |
| `ACCENT_INK` | 4.20 | 3.34 | **5.81 / 4.63** |
| `WARN_INK` | 4.12 | 3.28 | **5.82 / 4.64** |
| `GOOD_INK` | 4.70 | 3.75 | **5.83 / 4.64** |
| `BAD_INK` | 5.29 | 4.22 | **5.78 / 4.60** |

Four of those are under AA (4.5:1) on a panel and all of them are under it on the well. The fix
is eight bytes in `theme.rs` — each ink is the same hue, taken down until it clears the floor — and
the interesting part is that it stays fixed: `theme::tests::every_type_colour_clears_the_contrast_floor`
computes the ratio from the same sRGB curve the standard defines and walks the *inventory* of inks
against the *inventory* of surfaces, so a future retune of the cream cannot quietly make the
window's coloured type unreadable.

The pass also found three places where a **fill** was being used as **type**, which no amount of
ink retuning can fix: the status bar's warning line drew `WARN` (a lamp colour) on the sunk well
at **1.5:1**, the shelf's "open" badge drew `ACCENT` on a card at **1.5:1**, and an unloadable
level's reason drew `BAD` at **3.3:1**. They are `WARN_INK` (5.8:1), a filled chip of `ACCENT`
with `INK_ON_ACCENT` on it (9.8:1), and `BAD_INK` (6.2:1).

## Two kinds of line, and the states that had none

`EDGE` (a hairline between surfaces) was also the outline of everything pressable, at **1.53:1**
on a panel and **1.77:1** on a card, and the hover state was a gold rim at **1.48:1**. A resting
control therefore read as a word on a shadow and a hovered one was indistinguishable from it. The
palette now has both: `EDGE` stays decorative at 1.5:1, and `CONTROL_EDGE` is the boundary of
something you press — 3.0:1 on the well, 4.4:1 on a card, the WCAG 1.4.11 floor for a non-text
boundary. Hover is `ACCENT_INK` (5.8:1 on a card) rather than the accent fill, and the chart got
the same treatment with `SCREEN_EDGE` for its frame (3.0:1 on the plot) over `SCREEN_GRID` for
the rules inside it (2.2:1) — three rules at the frame's weight had no floor and no edge.

Two affordances came out of the same review. The shelf's cards were clickable only on their
thumbnail: the frame's own response is drawn-only, so a click on the name or the facts did
nothing. The whole rect is the control now, with a pointing hand and an accent-ink outline on
hover. And every lane's tag bar — eight pastel colours on whatever terrain the body is over —
gained a one-pixel dark halo, so a tag reads on pale sand as well as on a pit.

## What was checked and left alone

The graphics pipeline already does the things this pass would otherwise have added: the atlas is
uploaded with `TextureOptions::NEAREST`, the camera's origin and every sprite rect are rounded to
whole pixels before drawing, and zoom is snapped to an integer, so a sixteen-pixel tile is never
resampled into a smudge. The generated art is clean by the same kind of audit — no colour under
transparent pixels (which is what produces a halo when a sprite is filtered or composited), no
sprite over forty colours, no single flat colour anywhere except the ten `ui_*` glyphs, which are
deliberately one ink.

What a pass like this cannot do is *look*: the frames were verified structurally (each mode still
renders to a screenshot, and the composition's mean colour and colour count are unchanged within
noise), not by eye. The next pass should be a human looking at `--screenshot` renders of the four
modes beside these numbers.

# The playtest's feel: a body drawn between ticks, and a vine you can climb over

Three reports, one playtest: the body was jerky, a vine whose top was under a block could not be
climbed, and the oracle called a goal behind such a vine unreachable. The first is a view bug and the
other two are the same missing move seen from the sim and from the graph.

## Jerky: the body was smooth and the camera was not

The playtest already interpolated the body between physics ticks (`Session::body_at`) — 60 Hz physics
under a 144 Hz display, `prev + alpha * (current - prev)`. What it did not do was put the **camera** on
the same clock: `follow_player` ran after the canvas, from `sim.player()` rather than from the drawn
position. So the body was drawn at a point between two ticks while the scene behind it was drawn from
the last tick, and the body's offset from the centre swept a whole tick's travel and snapped back
every tick.

Measured on `l1-flat-run` at run speed, at 144 Hz, over 120 frames:

| reading | before | after |
|---|---|---|
| frames that drew a body which had not moved | 71 of 120 | **0 of 120** |
| step per frame, in tiles | 0 or 0.1000 | **0.0417 on every frame** |
| body's offset from the camera centre | sweeps 0.1 tile | **0** |
| that offset on screen, zoom 24 / 48 | 2.4 / 4.8 px sawtooth at 60 Hz | **none** |

0.0417 is exactly `0.1 × 60/144`: the accumulator's own share of a tick, which is what "the view is a
linear function of the simulation clock" looks like on a row of frames.

The second half was the animation. `Session::advance_actions` advanced the run cycle by
`ticks * TICK`, so on the 71 frames that ran no tick the legs stood still and on the 49 that did they
stepped — a 60 Hz run cycle under a body gliding at 144, which reads as a sprite skating on smooth
position. The animator is advanced by the frame's own **simulated** time now,
`frames * TICK + render_alpha * TICK`, kept as a clock and advanced by its difference; it is still the
simulation's time and not the wall clock, so a stalled frame that ran no tick still advances the
animation by exactly the fraction of a tick the body moved through. Two tests pin both halves through
what is actually drawn (`the_drawn_body_advances_on_every_frame_of_a_fast_display` and
`the_run_cycle_advances_on_frames_that_run_no_tick`, the latter counting sprite changes on frames with
no tick), because "smooth" is not a thing a screenshot can check.

What is *not* fixed, deliberately: the drawn position and the camera are still rounded to whole
pixels, so a slow scroll of less than a pixel a frame is quantised. That is the pixel grid the whole
game is drawn on — unrounded sprites at an integer zoom are the shimmer the renderer exists to avoid —
and it is a sub-pixel difference, not a frame-rate one.

## A vine's own block is a route, and it is a script rather than a jump

A vine that hangs beside a ledge is left sideways: climb to the top, press the side, land on the
ledge. A vine that hangs from a **one-tile-thick block** has no sideways exit at all — the body climbs
until its head meets the underside and then has nowhere to go, because the only way on is *over* the
thing it is holding. Holding climb there now hauls the body over that block.

The pull-up is scripted (`MANTLE_TICKS = 10`, a sixth of a second) for the reason a spring's throw is:
it is a traversal air control cannot express — it passes *through* the corner it is climbing over, so
collisions are ignored for its ten ticks — and a script that is identical every time is replayable,
sweepable by the oracle and cheap to animate. It eases on a smoothstep in both axes, so the body leaves
the vine slowly, crosses the lip quickly and settles onto the block rather than being placed on it;
its last tick reports `landed`, which is what makes the oracle's script end where a real landing does.

The trigger is the climb button the player is already holding — no new verb. The geometry is one
function (`vine_mantle_anchor`) rather than a rule the physics and the oracle each keep: it answers
"the block this vine hangs from, and is there room above it for a body", and it says *no* for a
two-tile lid, because there is no stand above its anchor and a pull-up there would put the body inside
a wall. That negative is a test as well.

## The oracle: the up-exit, and a flag that was "already home"

`MOVE_SET_VERSION` 7 adds the third vine exit to the sweep: `TakeoffDir::Still` climbs straight up with
no sideways press and lets the pull-up land the body on the block. It is swept only where
`vine_mantle_anchor` says the geometry is there, and it is an ordinary edge — `press_lands` asks the
same question of it, and `evo::route_play` executes it by holding `JumpInPlace`, which is the whole
move.

The graph fix that came out of testing it is version 8. The flag's own stand was the topmost stand
sharing the flag's **column**, which is true of a flag on a floor and false of one near the top of a
level: on the vine-block fixture the ground seven tiles below shares the column, so the graph answered
"you are already on the goal's stand" and the route walked a body to the flag's x on the floor instead
of up the vine. Only stands a body standing on them can actually *touch* the flag from count now; a
flag out of standing reach keeps its hop through the crossing that touches it in flight.

Verified at three levels rather than one, since each could pass alone:

* the sim — `holding_the_climb_hauls_the_body_onto_the_block_the_vine_hangs_from` (10 ticks of
  pull-up land the body standing, and the flag on the block is then walkable), plus the two-tile lid
  that must stay a dead end;
* the oracle — `the_oracle_reaches_the_top_of_the_block_a_vine_hangs_from`, which read *unreachable*
  before the up-exit;
* the replay — `the_replay_pulls_up_over_the_block_a_vine_hangs_from` in `evo`, which drives the plan
  through the real episode loop. The shipped table is unchanged: the ten levels in
  `crates/evo/tests/route_play.rs` walk exactly what they walked before, so the new exit is an
  addition and not a different route on old terrain.

What is still not measured: whether the ten-tick pull-up *looks* like a body hauling itself over a
lip, and whether the playtest now feels right to a person. Both are judgements, and the honest next
step is a human at the keyboard with `EVO_FRAME_LOG=1`, which prints every frame over two vsyncs.

## The furniture: what a Nintendo menu is made of, and what the window took from it

The title screen and the editor were built twice in one sitting: once in a palette of near-black
slate panels with gold keylines, which read as a *tool* with a coat of paint, and then again after
going and looking at how the games this one is descended from actually draw a menu. The second
version is in the tree. The rules it follows are conventions rather than specifications — Nintendo
publishes no style guide with hex values or radii — but they are consistent across *New Super Mario
Bros.*, *Super Mario Maker* and *Super Mario 3D World*, and the source material is the games
themselves: the NSMB manual's screen inventory ([manual](https://www.manualslib.com/manual/111808/Nintendo-Super-Mario-Bros-Wii-69151a.html)),
Maker's menu and palette structure ([Nintendo Support](https://en-americas-support.nintendo.com/app/answers/detail/a_id/15176/~/how-to-access-the-main-menu-%28super-mario-maker%29)),
and 3D World's edge-anchored in-course HUD ([manual](https://www.scribd.com/document/522690877/manual-WiiU-Super-Mario-3D-World)).

The six rules, and where each one landed:

* **No black.** Every outline is a warm chocolate (`#4A3420`), every panel a warm white (`#FFF9EB`),
  and the darkest surface in the window is the *level*, not the furniture. A neutral black keyline
  was the single change that made the first attempt read as a terminal; replacing it is most of the
  difference between the two versions. [`theme::OUTLINE`].
* **Colour is a material, not a tint.** Coin gold (`#FBC531`) is the accent and the dock's sign,
  ribbon blue (`#2B85D6`) is every heading, and the runner's vermilion is the menu cursor. Each is
  used for exactly one job, so a glance at a colour says what kind of thing it is.
  [`theme::ACCENT`], [`theme::RIBBON`], [`theme::RUN`].
* **Rounded plates with a thick outline and a hard shadow.** 6 px on a cell, 14 px on a panel,
  a 3 px outline, and a 3 px offset shadow with no blur — the depth is a printed plate lying on
  paper, not a modern floating card. [`theme::panel_frame`], [`theme::dock_frame`].
* **The selection is filled, and the cursor blinks.** The chosen menu line is a gold bar with a
  red triangle beside it blinking at 2.2 Hz, and the chosen tool is the same gold: "this is the one
  that is on" is one idea in both rooms.
* **A HUD is labels over values, in the corner, on a card.** The playtest's scoreboard is a warm
  white card with a gold hairline inside it: `TIME`, `YOUR BEST`, `TO THE FLAG` in blue caps over
  brown monospace values, drawn over the picture rather than in a bar of chrome.
* **Headers are ribbons.** Panel sections are blue tabs with white caps on a brown outline; the
  title screen's box has a `SELECT` tab stuck onto its top edge. This is what makes a column of
  sections read as a menu screen rather than as a document with subheads.

What is *not* verified: none of the numbers above are measured from Nintendo's output — they are
reconstruction values chosen to read correctly at this window's scale (a 40-point tool plate, a
21-point menu line). The contrast floors the palette already enforces still hold under the new
colours (`theme::tests::every_type_colour_clears_the_contrast_floor`), and the window was checked
by rendering a frame of each view (`--screenshot`, `--play`, `--evolve`, `--splash`), but "does it
feel like a Mario menu" is a judgement no test makes.

## Three ways a level and its window disagreed

Three reports, one theme: the picture in the window was not always the level the simulation was
running. Each is fixed in the layer that was actually wrong rather than papered over in the view.

**A chaser placed after a run started was missing from Evolve.** Edit and Play read the editor's
document; Evolve draws the run's own copy of the level, and it decides whether that copy is stale
with a hash. That hash was `Level::content_hash` — the terrain, spawn, patrols and flag — which
predates chasers, flyers, turrets and movers and never learned about them. Adding a chaser changed
the document but not the hash, so `Lab::trained_on` said "same level", the run kept its copy, and the
one view that draws a copy was the one view without the creature. The run now keys on
`Level::scene_hash`, which hashes every entity box as well; `content_hash` is untouched on purpose,
because it is the champion's and population's identity and changing it would invalidate every file
on disk. The window also draws the document's own entity placement in Evolve when there is no live
generation to draw — the first frame, a restart, a paused run — so an empty street is never mistaken
for a deleted enemy. Tests: `every_placeable_entity_changes_the_scene_hash_but_not_the_terrain_hash`
(sim) and `an_entity_placed_after_a_run_starts_makes_its_level_stale` (app).

**A moving platform's travel was invisible.** A shelf's span is a number in the file and the shelf's
own art only shows where it is *now*, which made placing one and timing a jump a guess. The view now
draws the travel under everything else: a ghost box at the far end of the **clear** span, and a dashed
band around the swept region. The far end comes from `physics::clear_travel`, the same function the
episode uses, so a shelf authored through a wall is drawn stopping at the wall rather than promising
a route the sim refuses. It is one pass in `draw_level`, so Edit, Play, Evolve and Replay all get it
for free and it is not drawn eight times over eight lanes. Test:
`a_platforms_travel_guide_ends_where_the_shelf_can_reach`.

**A stomped turret no longer exists, and a turret you could not stomp would be a wall.** The turret
was in the enemy pass: landing on it destroyed it and bounced the body, anything else killed. It is
now a solid box — its lid is ground, its sides stop the body, and neither is a death or a stomp — so
the only hazard it owns is the shot, which was always the point of the creature. The resolution runs
after the body's tile pass: a *crossing* from above lands on the lid (the same test a moving platform
uses, so a body falling beside it is not magnetised onto it), and a side overlap is pushed out along
the direction of travel. `stats.stomps` no longer counts a turret because nothing was stomped. Test:
`a_turret_is_a_solid_box_not_a_stompable_enemy` covers both halves and asserts the turret survives.

Docs corrected in the same pass: the README's enemy table, the editor's turret tooltip and the
`TurretSpec` comment all said "stompable", and the platform row now documents the travel guide.

## The vine let go of a body that was running at it

Reported from a run rather than a test: on a custom level whose only way across a lava pit is a vine
hanging from the ledge above its lip, the best agents kept failing **at the vine**. Their own episodes
show the shape of it — the population runs right, jumps at the pit, catches the vine and then slides
back out of the column into the lava.

**The trace is the finding.** The champion of that run, replayed against the level it was trained on,
was caught by the vine at `x = 20.62` carrying `vx = 0.09`, climbed **one tile**, and left the vine at
`x = 22.15` with nothing under it. Four of eight seeds died in the lava at `x ≈ 26.9` with **zero**
checkpoints; the other four only crossed when the policy happened to stop pressing right for long
enough on the climb. That is the whole bug: the grab caught `vy` and nothing else. The physics
comment has said since the elements pass that "arriving at a vine at speed catches the body on the
first tick", and that was true vertically and false horizontally — a body arriving at a run kept the
run's speed, the column is one tile and the body 0.8 tiles wide, so the hold band is 1.8 tiles and a
direction held for a dozen decisions carried it out. And the direction a policy holds *is* right:
every other part of every level rewards it, so the vine was the one place where the locally correct
action was fatal.

**The fix is that the vine holds.** The grab spends the body's horizontal speed, the same way it
already spent the vertical one, and while the climb is held below the top of the run the vine owns
`x`: a direction is not a walk. "Hold jump and keep running" climbs now. The last
`physics::VINE_EXIT_REACH` (0.9 tiles) below the top is the **exit window** — the sideways press that
carries a body onto the ledge beside the run, which is also exactly where
`reach::vine_action` presses — so stepping off onto a ledge is unchanged, and releasing the climb
(below the window) still lets a side press step off mid-shaft, which is how a body drops out of a
vine it no longer wants.

**The oracle is untouched, and that is the check that the change is in the right place.** The
scripted vine climb presses no direction until the exit window, so every route the oracle walks is
bit-identical: `reach`, `evo::route_play` and the ten-level verdict table all pass without a number
moving, and the custom level still answers "reachable". What changed is only what a *policy* does
when it presses into a vine. The version trio moves to **sim 0.15.0** because a champion's physics
is not the same any more; sensors v12 and fitness v8 are untouched (the patch already said where the
vine is, and the reward already paid for ground).

Tests: `a_vine_catches_a_running_body_and_holds_it_while_the_climb_is_pressed` (sim) is the reported
case as a fixture — a vine over a lava pit, the run button held the whole way, the body caught at the
lip, the column held to the top, the pull-up ending standing on the ledge; on the old rule it fails as
soon as the body slides, which is the second tick of the climb. The existing vine tests (climb and
slide rates, the sideways exit, the pull-up,
the two-tile lid, the oracle) pass unchanged, and `route_play`'s vine fixture still walks.

**Measured on the level that produced the report**, pop 200, `--stop-on-win`, 200 generations, the
same command and seed either side of the change:

| | first win | detours needed to get there | clock |
|---|---|---|---|
| vine let go (before) | **generation 141** | **3** — the first at the vine itself: spawn tile 17, flag on the ledge at `(21, 3)`, 38 generations to learn one climb | 5.7 s of search |
| vine holds (after) | **generation 13** | **0** | 0.2 s of search |

The first detour in the before arm is the whole finding in the machine's own words: the stall chain
derived a practice level whose only move is the vine, and spent 38 generations on it. That detour no
longer exists. The 10× is a reading of this level rather than a law — a first win is a threshold and
an easy one at that — but it is the difference between a level whose only route is a trap and one
whose route is a move.

**What this does not fix.** The level that produced the report still has a turret on the ledge at
`(28, 3)` firing back along the row the body stands in after the pull-up, so the climb can end in a
shot rather than a fall. That is a level-design note rather than an engine one — a turret covering
the only landing is a decision the author makes — and the fresh population above solves the level
anyway (8.25 s at the flag on its first win). What it does mean is that the champion saved from the
*old* run is no better after the fix than before it: its policy was trained on a vine that let go,
and it alternates climb and brake at the top instead of pulling up.

## A custom copy of a shipped level was writing over the pack's champion

The second report from the same run, found while reading the files it left: the pack's
`champions/l1-flat-run.json` had been replaced by a champion whose level hash is the *custom* level's.
The chain is three files that each did the reasonable thing: `protect_shipped_path` saves a copy of a
shipped level as `custom-<stem>.json` while keeping the shipped level's **name** (that is what a copy
is), the shelf and the title screen look for a champion beside the level under the **file's** stem,
and `train-cli` names its output after the file's stem — but the window's `Lab::stem()` slugged the
level's *display name*. The two spellings agree for every shipped level and for every level somebody
names by hand; they disagree exactly when a level is a copy of another, which is the commonest way a
person starts one. A run on `custom-l1-flat-run.json` therefore wrote `champions/l1-flat-run.json`,
and `ChampionFile::load_for_level` then refused that file for the shipped level it now disagreed with.

`lab::artifact_stem` is the one statement of the rule: **the document's file stem when it has one, and
the slugged display name only for a document that has never been saved** (where the first save will
wear that name anyway). `Lab::start` and `Lab::restart` take it, the app computes it from
`editor.path`, and the run's champion, practice level and checkpoint all wear it, so the window and
the terminal now name the same artifacts for the same file. Test:
`a_custom_copy_of_a_shipped_level_names_its_artifacts_after_its_own_file`.

The clobbered file is not restored: it was already committed, and under sim 0.15.0 every champion on
disk is refused anyway. The next run on the copy writes `champions/custom-l1-flat-run.json` and the
pack's own champion is retrained or left alone, which is the same bill the version bump sends to every
other file in the directory.

## The window kept three copies of the level, and New only replaced one

The report from the window: *"the levels buttons show the levels, but does not let me select/load
them"* and *"when clicking New … it keeps whatever you were working on before"*. Both were
reproduced through real presses, in a harness that did not exist before: `EvoApp::build` now takes a
bare `egui::Context` instead of an `eframe::CreationContext`, and the frame half of
`eframe::App::update` is `EvoApp::frame`, so a test can run the real window — the dock, the shelf, the
fade, the canvas — and click it with real pointer events. (The frame helper advances `time` a
sixtieth per frame; a harness that fed every frame `time = 0` makes egui treat every click as the
second half of a double click.)

The measurement is three presses:

| press | before | after |
|---|---|---|
| **New** on a clean document | works | works |
| **New** inside a playtest | `mode=Play session=true drawn_is_old=true` — the old level still running | `mode=Edit`, no session, canvas on the new document |
| a card on the shelf | opens it | opens it |

**The cause of the first is that a document is not the only copy of a level in this window.** A
playtest session, a replayed episode and a running population each hold their own copy of the terrain
they started on, and while one of those views is up it is *that* copy the canvas draws (`viewed_level`
and the Play arm of `canvas`). **New** swapped `editor` and left all three in place, so in Play or
Evolve the level on screen did not change at all — only the plaque did. `open_level` dropped the
session but not the run or the replay. One path now does both (`EvoApp::apply_replace`): install the
document, drop the run (its worker joined on its own thread, so New does not freeze the window for
the length of a generation), drop the session, the replay, the lanes and the packet, return to Edit,
refit the camera, and reset the rename buffer.

**The cause of the second is that a refusal was a whisper.** The two-press rule for a dirty document
is unchanged and still pinned by `discard_guard`'s test, but its only voice was a sentence in the
status line at the bottom of the window — which is why a card that asked a question and a card that
did nothing looked the same. The same refusal now also puts a card above the status bar, naming what
was asked for (*Opening l2-first-gap.json would throw away this level's unsaved edits.*) with the two
answers on it: **Discard** and **Keep editing**. The card is placed *below* the middle of the window
on purpose, so it never covers the card that is being asked about — pressing it again is still the
other answer — and a save clears the question, because the work it was protecting is on disk.

Two more defects fell out of driving the real controls:

* **A double click was the pointer's, not the card's.** egui's `double_clicked` tests the clock and
  nothing else, so *any* click within 0.3 s of another reports a double click on whatever it landed
  on — and the shelf turns a double click into a playtest. Clicking **Levels** and then a card at
  speed dropped the viewer into the playtest instead of the editor. The shelf remembers `(card,
  time)` itself now.
* **The dock moved on the first stroke.** The plaque is as wide as its own words, and its second line
  grew from `file` to `● file — not on disk` when the document became dirty, pushing every tool to
  its right sideways and pulling them back on the save. Both states are two columns and a file name
  now — `● ` or two blanks — and the sentence lives on the plaque's hover.

Tests: `the_new_button_clears_the_level_and_every_view_of_it`,
`a_card_on_the_shelf_opens_its_level_and_a_dirty_document_is_asked_first`,
`the_discard_buttons_answer_the_question_they_are_asked`,
`the_dock_does_not_move_when_the_document_becomes_dirty`, and
`a_quick_second_click_opens_a_different_card_and_double_click_plays_the_same_one`.

## A vine under a one-way shelf was hanging off nothing

Reported from the editor: a shelf sometimes has a vine under it and the vine does not reach the
platform, so it reads as a cutting stuck in mid-air rather than a plant growing out of the shelf.

**The art is not at fault, and its own notes say so.** A one-way platform is a thin timber deck
across the *top* of its cell — three of the sixteen rows a cell is drawn in, with iron brackets
under those and open air below them (`tools/art/terrain.py::oneway`) — while everything that hangs
in this game was drawn to start at the top of its own cell, which it assumed was the underside of a
ceiling. `tools/art/props.py::vine` puts that rule in words: *"Its row 0 is opaque on purpose: a vine
whose stem began a pixel below the top of the cell would look like it was floating under the
platform rather than growing out of it."* A shelf breaks the assumption, because its cell is mostly
air: the cell boundary is 13/16 of a tile below the wood. Both vines in the level were therefore
drawn one deck too low, and both are fixed in the view, with the drawing untouched:

* **The scattered hanging vine** (`hanging_sprite`, one column in six under a ceiling) is a `Prop`
  with a new `hangs` flag. A standing prop is placed by its feet on the ground line; a hanging one is
  placed by its *top*, on the underside of what it grows out of — which `draw_prop` now reads off the
  level, because under a shelf that underside is the plank and not the cell boundary. Under a solid
  ceiling the two lines are the same, so those vines are drawn exactly where they were.
* **A painted shaft of `Tile::Vine`** has no cell to spare above it — the shaft already fills its own
  cells and tiles vertically, which is what makes ten of them one plant — so the missing strip of the
  shelf's cell is filled with a crop of the vine's top rows (`draw_vine_to_shelf`), in the same
  columns and at the same pixel scale as the cell below. Cropped rather than stretched: the stem is
  continuous from row 0 to row 15 by design, so the join is invisible.

`ONEWAY_PLANK` (3/16 of a cell) is the number both paths need, and it is pinned to the shipped
atlas rather than trusted: three of the sixteen rows of every `oneway_*` sprite fill the cell, and
the next one does not, which is where the brackets start.

**What this does not change**: the deck stays thin (a one-way platform that filled its cell would
look like a wall the body walks through — the thing the tile exists to avoid looking like), and a
vine keeps its own length. It is attached higher, not stretched longer. Placed scenery obeys the same
rule, so an author who drops `vine_a` under a shelf gets the same connection the scatter gives them.

Tests: `a_scattered_vine_hangs_from_the_shelf_it_is_tied_to` and
`a_painted_vine_under_a_shelf_reaches_the_plank` (app, drawn through the headless frame harness and
measured against the shelf's own quad), and `a_one_way_shelf_deck_is_the_top_rows_of_its_cell`
(atlas, which pins `ONEWAY_PLANK` to the drawing). The scattered test carries a rock ceiling beside
the shelf in the same level, so the vines that already looked right are asserted to have stayed put.

## The verdict called a level with a shuttle in it unbeatable

Reported from a custom level: *"the oracle doesn't class the level as finishable because it doesn't
properly account for the moving platforms"*. It was right on both halves. The level is a sixty-tile
run over a lava floor with two shuttles carrying the body across it, and `reach` answered
**UNBEATABLE — no canonical move from platform 3 reaches the next surface**: the spawn stand is the
four-column ledge at the left, every move off it lands in the lava, and the shelf that crosses the
lava was not in the sweep at all.

**Why it was not, and why the reason did not survive contact with a level.** A mover is not terrain.
Its phase is a fact about the *episode* — a shelf is at a different place on frame 0 than on frame
90 — and the oracle's move set is a statement about the level, so the sweep simulated the level as
if no platform existed and the level file's own note was where the author said what that meant.
That is a defensible position about a *route*, and it is the wrong answer to the question a verdict
is asked. "Can this level be beaten?" is true of a level whose only crossing is a shelf, and the
person who drew one is entitled to hear yes.

**The fix is a ride, modelled as a ride.** A mover's travel has two ends, and both are stands: the
deck parked at the near end and the deck parked at the far end, each at the deck's own half-cell
height (`Platform::surface_y`, which is why that field is a float — a deck's top is mid-cell, and
rounding it to a tile edge would be a half-tile lie in every jump off it). Between the two there is
one edge the sweep cannot simulate and does not need to: *wait for the cycle*, which is always
available because the cycle never stops. `reach_platforms` therefore answers the verdict's question
with the terrain **plus both ends of every mover**, and `Platform::ride` names the partner stand.
The level now answers OK with 11 surfaces, all reachable, where it previously answered with 5 and a
sealed spawn.

The route graph deliberately still does not model it (`goal_routes` keeps the terrain table). A
*plan* has no way to press a clock, and the waypoint sensor would steer a body that is already
riding at the far end of the travel, which is the one place it must not walk. The consequence is
recorded below rather than papered over.

## The editor could not say which way a shelf leaves

A turret has carried a firing direction since it existed, in the level file, in the palette row and
in the mirror; a shelf carried only its width, its travel and its axis. So a level whose only ride
*arrives from the right* could not be drawn, and the two shelves that differ only in direction were
the same three numbers in a file. `PlatformSpec::dir` is that field now — `+1` right (or up, for a
lift), `-1` left (or down) — and it is threaded through everything a direction has to reach: the
loader (`axis: "down"` still means what it always meant, and a file with no `dir` is the rightward
shelf it always was), the writer, `Mover`'s travel and its state hash, `clear_travel`'s probe, the
whole-document mirror (**a slider's direction turns with the level; a lift's does not**, because
up/down is not the mirrored axis), the palette row (`goes: left | right`, or `down | up` for a
lift), the eyedropper, and the placement call.

Tests: `a_shelf_travels_the_way_the_file_says_it_does` (both spellings, through the real loader and
the real stepper), `a_mirrored_shelf_travels_the_mirrored_way` (the slider turns, the lift does not),
and the writer round-trip in `every_new_entity_round_trips_through_the_writer`.

## The agent stalls at the shuttle, and the three things that did not fix it

The same level, trained: **5 seeds x 200 generations x pop 400 solved it 0 times**, best `x` between
40.9 and 52.4 — past the first shuttle, dead on the second. A first fix pass moved the goal posts
(respaced spikes, below) and one of five seeds then won at generation 133. At **600 generations x
pop 600, 0 of 5** — so this is not a budget problem.

**The arithmetic is the finding.** A mover travels at `mover_speed` = 0.0625 tiles a tick, so a
twelve-tile crossing takes **192 ticks each way**; the deck is three tiles wide and a body walks at
0.10, so a policy that holds right — the correct press on every other piece of ground in the game,
and the press this one has learned — walks off the front of it in **30 ticks**. Crossing the level's
two channels therefore means standing still for roughly 56% of a winning episode, and the
observation cannot say so: a deck is drawn into the patch as `ENC_ONEWAY`, the same value as a fixed
one-way shelf, and nothing in the vector says the ground is moving.

**Two sensor changes were built, measured and reverted.** v13 gave a deck its own patch value and two
slots for the velocity of the deck under the feet; at 200 x 400, five seeds, that arm won **0 / 5**
(best `x` 46.3, 42.3, 37.2, 46.4, 41.8) against the unadorned build's **1 / 5**. The same slots with
the patch value left alone: **0 / 5** (40.1, 46.1, 43.7, 44.7, 37.1). A wider vector is a larger
search space and two extra inputs were not worth eight generations of progress on this level, so
neither arm shipped. What the numbers say is narrower than "the sensor is wrong": the information is
real, and at this budget its cost is larger than its value.

**What the run is actually starved of is the curriculum.** Every stall in every seed logged the same
sentence:

```
curriculum: no crossing toward the flag from tile (31, 11) in L1-flat-run; the run stays where it is
```

Tiles (31, 11) and (32, 11) are the ledge the shuttle leaves from. `practice_level_for_stall` derives
its practice level from `next_stand`, which asks the *route graph* for the next stand toward the flag
— and the graph was terrain-only, so from that ledge the answer was "there is no route at all". The
one mechanism in this project built to rescue a stalled run was switched off for exactly the levels
the verdict had just been taught to trust. The ride went into the graph first, and the flag it
derives went to the wrong crossing second; that story is the next section.

## The practice flag went to the crossing the run had already made

**The ride is in the route graph now.** `goal_routes` builds a stand for each end of every mover's
travel and links the pair with `Stand::ride` — the one edge in the graph nobody presses, because the
cycle makes it true — so `next_stand` from the ledge a shuttle leaves answers the deck instead of
"no route", and `GoalRoutes::aim` answers a ride with a waypoint on the body itself, which is the
only honest way to say *hold still* in a vector whose two slots are a direction.

**The flag still went backwards, and the reason is a spawn is not a stand.** `stall_spots` returns
two answers, because a stall can happen on a deck and only one of them is a tile a spawn may be
placed on: the deepest stand the genome has **stood** on (terrain or deck) and the deepest *ground*
tile. The practice level took the tile and asked `next_stand` about **it**, so a run that had ridden
a shuttle and died on the deck past it was measured from the ledge two decks back. Observed chain,
on the reported level:

```
curriculum: 40 generations on L1-flat-run practice at tile (16, 12) to tile (18, 12) ...
curriculum: 40 generations on L1-flat-run practice at tile (16, 12) to tile (18, 12) ...
```

— the spike jump the champion made fifty frames before it died, twice, while its death was at `x`
35.8, two decks along. `practice_level_from(main, tile, Some(stand))` is the flag measured from the
deck while the **spawn stays on ground**; `practice_level` is that with `None`, unchanged. Then one
more candidate was removed on measurement: the far end of the *same* deck's travel. The graph
over-approximates a same-deck hop (a take-off platform is static and a landing deck is at both
ends), so from a deck's near end the flag landed on its own far end — a "crossing" that is just the
ride it is already on, and the practice level taught nothing.

The chain on a seed that solves the level is the whole argument, and it is why the fix is worth its
three lines: every one of those generations is inside an excursion, and the last flag is the finish.

```
practice (18, 12) to (19, 12)   flag reached at its generation  35
practice (18, 12) to (19, 12)   flag reached at its generation  36
practice (16, 12) to (18, 12)   flag reached at its generation  76
practice (16, 12) to (34, 12)   flag reached at its generation  90   <- the deck crossing
practice (18, 12) to (50, 12)   flag reached at its generation 269   <- the run's own flag
```

**Measured, and the honest half of it.** 8 seeds, pop 300, 600 generations, memoryless,
`--stop-on-win`: **2 / 8** with the flag measured from the frontier stand (seeds 1 and 2, first wins
at generations 269 and 311) against **2 / 8** with the flag measured from the spawn's stand at the
same budget (seeds 2 and 7, at 168 and 382). So this is a correctness fix and not yet a win-rate one
at eight seeds — which seeds win moved, and the chain above is the case it repairs. The two obvious
next things were measured and are not the answer: `--feedback 8` won **0 / 8** and `--feedback 16`
**0 / 8**, agreeing with the reverted v13 arms that a wider vector or a recurrent state costs more
search on this level than the information is worth at this budget.

**A solved champion sits beside the run's own** (`champions/` is ignored, like every champion file).
`champions/custom-l1-flat-run-solved.json` — the
seed-1 genome above, won in 845 frames with 12 jumps and 1 checkpoint, `1 of 1 trials won`, and
`replay` verifies its trajectory bit for bit. The level's own champion, trained for 93 generations
before any of this, dies at `x` 35.8.

**What is left is the trigger, not the flag — and the obvious trigger is a loss.** The chain above
only moves at a stall (twenty-five generations of no new best, `render::chart::STALL_GENERATIONS`),
and every win above arrives inside an excursion, so the next thing tried was to make practice fire
sooner. `STALL_GENERATIONS` at **8**, same eight seeds and budget: **0 / 8**, with the best `x`
*better* than the baseline arm's on five of the eight seeds (33.9, 31.8, 48.4, 48.4, 35.7 against
29.4, 29.4, 32.4, 33.2, 32.3) and every one of them still short of the flag. More excursions is not
more learning — a run that stops every eight generations to rehearse has fewer generations on the
level to use the rehearsal in — so the constant went back to 25 and the frontier trigger (practise
the link the moment it *is* the frontier, REVIEW.md item 6) stays an unmeasured idea rather than the
next fix. Measured and rejected in the same pass:
`--novelty 16` (1 / 8) and `--archive-on-diversity` (2 / 8, the gate never firing on a level that
keeps finding new niches).

Tests: `a_practice_crossing_off_a_deck_is_measured_from_the_deck` — a shuttle between two ledges, with
the same run asked twice: from the ground the flag is the jump onto the deck, from the deck it is the
ground past the ride, and the spawn is ground both times because a deck is not a tile a spawn may be
placed on.

## A one-tile landing pad is not a crossing, and the oracle cannot see that

The level's finish was a sand shelf with `^.^.^` on it: three spikes two columns apart, leaving
**one tile of safe ground between each pair**. Every seed died there or just past it — best `x`
50.4 and 52.4, which is standing on the first pad. Respaced to two-tile pads (`^..^`), the same
seeds reach the flag.

The probe that explains it: a spike at the body's own standing height has to be cleared by a full
tile of rise, and a canonical trajectory holds one horizontal input for the whole flight. The body
needs 3.5 tiles of travel by the tick its feet come back down to `11.0` — and at that tick it has
flown 3.53 tiles, so the arc into a one-tile pad clips the *next* spike with the body's trailing
corner by a tenth of a tile. Two tiles of pad is comfortable; one is not, for a straight-line jump.

**A policy can do it, because a policy has air control** — it releases the direction mid-flight and
drops on the pad, which is why `scratch-spike-tight` (spikes two columns apart across a whole floor)
is solved 11 times in 24 by the population while the oracle's move set cannot take a single pad. So
splitting the verdict's surface table at spikes — which looks like the honest fix, and was built —
**turns a solvable level into a false negative**: `scratch-spike-tight` came back unbeatable and two
`train-cli` tests failed on it. That split is reverted, with the reasoning left on
`extract_platforms`, and the gap is the deferred item below. The level was respaced for the
*agent*, not to make a verdict agree.

Tests: `a_shuttle_across_lava_is_the_crossing_the_verdict_reports` — the fixture is the reported
level in miniature: two ledges, a lava channel too wide to jump, one shuttle. The verdict must say
yes **with** the platform and no **without** it, so the first answer cannot pass by some other
route.

# Solved, then validated: what a champion is worth, and how a verification set can lie

`EpisodeResult::won()` asked one question — did the *representative* episode reach the flag — and
everything downstream inherited it: the generation's `wins`, `--stop-on-win`, the panel's solve
claim, and the champion a run saves. On a walker level the representative episode is trial 0, one
drawn patrol phase out of a continuous family, so the run could stop on a quiet phase and save a
policy that loses the next one. The conservative reading existed (`suite_wins`, every level won) but
was gated on `levels > 1`, so a *single* walker level had no such signal at all.

The fix is a second reading on the same tally. `SuiteTally` already folds every trial it records,
so it now also sets `EpisodeResult::all_trials_won`, exposed as `solved()`: **every episode this
genome was actually judged on reached the flag.** `won()` keeps its narrow meaning because a
champion file records one episode and `replay` reproduces it; `solved()` is what the generation
report, `--stop-on-win`, `bench` and the panel's strict line read. On a deterministic level the two
are the same value by construction — one episode is its own whole — so no flat-level number moved.

That alone was not enough, and the measurement says so. Four seeds of `chain` (`l9-wait-and-climb`,
pop 200, 400 generations, `--stop-on-win`), then each saved champion re-scored on twelve fresh
master seeds (101–112):

| seed | solved at (generation) | champion won, fresh episodes |
|---|---|---|
| 1 | 44 | 4 / 12 |
| 2 | 48 | 12 / 12 |
| 3 | 33 | 4 / 12 |
| 4 | 42 | 2 / 12 |
| | | **22 / 48 (46 %)** |

The champion is `best_ever`, and `best_ever` was still the generation's **best score** over draws.
The first repair tried was the obvious one: four fixed validation episodes (`validation_seed`), and
the challenger kept the record when its mean was higher. It made things *worse*, and the failure is
worth recording because it is a trap a validation set sets:

* seeds 1, 3 and 4 saved a champion whose own recorded episode was a **death** (gen 42, frame 109,
  `best_x` 17.3) while the generation that stopped the run had a genome that won all three of its
  trials;
* the fresh-episode count stayed at 22 / 48, so the held-out set bought nothing;
* a four-episode mean cannot rank a genome that won three and died on one above one that reached
  mid-level on four — and with enough challenges the maximum over four-episode means is itself a
  luckiest-draw statistic.

Two changes fixed it, and both are about **what a champion is**:

1. **`BestEver::validated` is a `Validation { wins, episodes, mean }`**, not a mean. Records are
   compared on `(solved, wins, mean)`, lexicographically (`Validation::beats`), so a genome that
   won the set beats one that merely scored well. Twelve episodes became the default
   (from four) — still only paid when a record is challenged, and still skipped on a level with
   nothing stochastic in it.
2. **Solved first, always** (`supersedes`): a record whose own episode is a win may not be replaced
   by one whose episode is a loss, whatever the score or the held-out mean. This is the invariant
   the `replay` command has always implied — the champion is a demonstration — and it was the
   thing the stop/save pair was quietly breaking.

And then the stop itself. `train --stop-on-win --stop-on-validated` stops when the **record** has
won every validation episode, not when a generation has won its own three trials. Measured on the
same four seeds and the same twelve fresh episodes:

| seed | solved at | held-out reading | champion won, fresh episodes |
|---|---|---|---|
| 1 | 70 | 12 / 12 | 12 / 12 |
| 2 | 48 | 12 / 12 | 12 / 12 |
| 3 | 61 | 12 / 12 | 12 / 12 |
| 4 | 62 | 12 / 12 | 12 / 12 |
| | | | **48 / 48** |

The cost is a median of 23 extra generations per seed (44→70, 33→61, 42→62; seed 2 was already
validated at its first solve) and 1.3–2.6 s of wall clock. The validation seeds cannot coincide
with the scoring seeds: `validation_seed` draws on generation `u32::MAX` and genome id `u32::MAX`,
which no issued genome and no generation reaches, while `score --seed m` draws on generation 0.

What is deliberately **not** changed: `bench`'s first-win stop and every table measured with it. An
arm that uses the strict stop records `stop_on_validated` in its file, the header says what it
stops on, and `--compare` refuses two arms that differ there — a three-phase draw and a
twelve-world verdict are different measurements, and a table that mixed them would be the kind of
quiet inconsistency this file exists to prevent.

Tests: `solved_requires_every_trial_where_won_asks_only_the_representative_one` (the two readings,
including a single-episode evaluation where they must agree);
`a_solved_reading_supersedes_a_higher_scoring_unsolved_one` (the ordering, including the regression
above); `the_record_carries_a_held_out_reading_only_where_the_world_moves`;
`validation_episodes_are_fixed_and_belong_to_no_generation`; `the_validated_stop_needs_the_win_stop_and_a_validation_set`.

# The archive earns its place when its children stay local

The archive was measured as a loss and shipped off by default. That measurement was taken with
coarse children: eight archive children per generation bred with the ordinary move (~86 genes at
`--mut-rate 0.05`, sigma 0.30), which is exactly the perturbation that destroys the behaviour the
incumbent was kept for. Breeding them with the fine step — one to three genes, `fine_sigma` — turns
the mechanism from "a below-average parent with a random child" into a hill climb inside a niche.

Eight seeds of `chain`, pop 200, 400 generations, `--stop-on-win`, each arm one `bench` run and
compared against the default by seed:

```text
train-cli bench --protocol chain --seeds 8 --arm plain --out /tmp/arms
train-cli bench --protocol chain --seeds 8 --arm archive-fine --archive-children 8 --out /tmp/arms
train-cli bench --compare /tmp/arms/chain-plain.json,/tmp/arms/chain-archive-fine.json
```

| arm | solved | median first win |
|---|---|---|
| no archive (default) | 4 / 8 | 238.0 |
| `--archive-children 8`, fine children | **6 / 8** | **147.0** |
| `--archive-children 8`, `--fine-sigma-scale 1.0` | 5 / 8 | 99.0 |

The paired comparison of archive-on against archive-off is **6 gains, 0 losses, 2 ties, p = 0.031**
(two-sided exact sign test). The fine step inside the archive is *not* established at this size
(4 wins to 2, p = 0.69), and neither is the fine step on ordinary fine children on its own
(5 / 8 vs 4 / 8, 3–4, p = 1.0, median 150 against 238). So the honest claim is: **the archive is
worth its eight slots once its children are local**, and the fine step is kept for the reason it
was introduced — a niche's behaviour survives a step and does not survive a jump — not because
this arm proves it. Twenty-four seeds would be needed to say more, which is the bar the review set
for a default and this pass did not meet.

Two control readings worth keeping: two arms with identical configurations produced identical
per-seed rows (the trainer is reproducible through `bench`), and `--fine-sigma-scale 1.0` is the
pre-fix behaviour, recorded in the arm file so the old measurement can be reproduced rather than
remembered.

# Four rows at a time, at the same bits

The forward pass was a chain of dependent adds: `acc += w*x` waits for the previous `acc`, so a
row costs its length in add latencies however fast the multiplier is. `dense` now computes four
output rows side by side (`DENSE_ROW_BLOCK`), each with its own accumulator and its own
left-to-right order.

Measured on the baseline machine, `96 → 16 → 11` (1 712 multiplies), 200 000 forwards, release
build, three runs each:

| loop | ns/forward | cycles/MAC at 2.9 GHz |
|---|---|---|
| one row at a time | 1 778–1 788 | 2.90–2.92 |
| four rows blocked | 706–716 | 1.15–1.17 |

That is **2.5×**, and the reason it is allowed in a repository whose determinism is a contract is
that it is a *throughput* change only: each row's sum is untouched, so every score is bit-identical
and no champion, table or version moves. The test that carries it
(`the_blocked_layer_is_bit_identical_to_one_row_at_a_time`) keeps the pre-change loop as the
*definition* and compares `to_bits()` over five shapes — four full blocks plus a three-row tail
(the shipped 11 outputs), all blocks (32 → 16), and 1, 3 and 5 hidden units, which are all tail. A
mis-sized block or an off-by-one in the tail is a test failure rather than a champion that stops
replaying.

What this does **not** say: the end-to-end generation speedup. The forward pass is about half of a
generation (review §2.5), so the expected saving is in that range, but it was not re-measured here
and the number to quote is the microbenchmark above.

## Deliberate play: the agent that looks at the world before it moves

The GA is a search with no model: every genome is judged by an episode that has already
finished, so a deck that is not under the body *now* is indistinguishable from a deck that
never arrives. That is the shape of the l11 failure above, and it is not repaired by more
generations. The deliberate controller is the other half of the agent. It is not a policy
over observations; it is a search over **the real simulator's next few decision blocks**.

`evo::navigate::play` holds the real `Sim`, asks the route graph where the next landing is,
and expands every action for a horizon of decision blocks. Each node is a cloned sim, so a
jump into a pit is a death the search has already seen rather than a score it has to infer.
The score is the sim's own progress toward the flag, the rungs left, and survival; the
keep-one-node-per-first-action rule is what lets a jump survive pruning while its first
blocks score worse than waiting. `Driver` holds one action for the four-tick block, so what
a window plays is the episode the search reasoned about. The whole-route timed planner is
the other mode (`--plan`): it searches the deck cycles and cooldowns that a six-block
horizon cannot see, then replays the route before accepting it.

Measured on the shipped pack (release binary, seed 1, local `--horizon 12 --beam 24`):

| level | local controller | timed plan |
|---|---|---|
| `l1-flat-run` | won | won, 93 decisions / 371 frames, 0.0 s |
| `l2-first-gap` | won | won, 93 / 369, 0.1 s |
| `l3-gauntlet` | **won**, 1733 frames, 1.7 s | won, 112 / 446, 0.2 s |
| `l5-tightrope` | died at x 54.3 (spike) | won, 197 / 788, 0.2 s |
| `l9-wait-and-climb` | **won**, 693 frames, 0.9 s | **no route** after 128 expansions (frontier x 37.4) |
| `l11-the-elements` | died at x 52.7 (lava, on the second deck) | **won**, 240 / 957 frames, 2.9 s |

The two modes are complementary rather than redundant: the local horizon untangles a patrol
that the whole-route beam prunes (`l9`), and the whole-route planner waits out the deck
cycle that is outside the local horizon (`l11`). The window uses the plan when there is one
and falls back to the local controller when there is not, so `--think` / `T` answers both.
Against that, the same l11 at 300 generations, pop 200, three seeds: **0 wins**, best `x`
46.2 / 48.1 / 50.6 — the controller is the difference between a policy that has to
discover a wait by mutation and one that can simulate it.

What this is **not**: a learned policy, a replacement for the GA, or a claim about the
evolved champion. It is the deliberate layer the plan's WP6 asked for; the evolved agent
remains the fast reflex. The window exposes it on `T` in any playtest and on
`evo-app --play --think`, and the terminal on `train-cli think` (`--plan` for the
whole-route mode).

# Deliberately deferred

**A hop in the canonical move set.** A policy clears a one-tile pad by releasing its direction
mid-flight; a canonical trajectory holds one input for the whole flight, so the oracle cannot express
it and must keep treating a spiked run as one surface. The missing move is a second hold stage — fly,
then release (or oppose) — which multiplies the sweep by the release tick and would change
`MOVE_SET_VERSION` again. Until then the verdict over-claims on a spike run and the honest statement
is the note on `extract_platforms`.

NEAT/speciation, survival-analysis difficulty estimation, online champion
sharing, sound, mobile, powerups, achievements/tutorial, threaded WASM, and the
web build. See the plan's §12 for the gates.

No longer deferred: the pixel renderer, the editor and the watch mode, and **the ride in the route
graph** — deck stands, `Stand::ride`, the hold-still waypoint and a practice flag measured from the
deck all landed with the finding above. Still deferred inside it is the *trigger*: practice fires at
a stall, not at the frontier. All three
existed as a terminal view and hand-written JSON through M1–M3, and are now a real
window (`crates/app`) — which means a non-developer can build a level, validate it,
play it and **evolve a solver on it without touching JSON** (the last third of M4's
exit criterion, in place as of finding 26). Still open inside it, in the order worth
doing: the curated starter pack M4 exits on (built with the editor, not by hand) and the
level list as a browsable shelf rather than an Open menu. The watch mode's last
terminal-only feature — the stall fallback of finding 22 and the resume that goes with
it — is in the window as of finding 28, so every M3 and M5 watch behaviour is reachable
from both front-ends.

# Smarter, not slower: phase 1

October 2026. The first pass over [SMARTER_AI_PLAN.md](SMARTER_AI_PLAN.md), whose
rule is evidence before code. The plan's first tier is Item 1 (run the gates on
the features that already exist), Item 2 (a batched forward pass), Item 5 (early
termination) and Item 3 (sensors v14). This section records what the first two
told us, the two pieces of code that do not depend on their outcome, why sensors
v14 was built and measured in that pass and then removed from the tree, and why
Item 8's narrower phase draw has no lever in this build.

## Item 2: the forward pass is a quarter of a generation, not half

The plan projects that the forward pass is "about half the cost of a generation"
and that batching it should cut generation wall time by at least 25 %. The
measurement is in `crates/evo/tests/performance.rs`, which is `#[ignore]`d
because it measures a machine rather than asserting a result:

```
cargo test --release -p evo --test performance -- --ignored --nocapture
```

On the baseline machine (i5-9400, six cores, release, idle), `l9-wait-and-climb`
at pop 200, generation 0:

| shape | forward | decisions | forward CPU | share of the generation |
|---|---|---|---|---|
| 108 → 16 → 11, memoryless | 885.7 ns | 24 503 | 0.022 s | **26.0 %** of 0.014 s × 6 cores |
| the same with feedback 16 | 972.9 ns | 24 503 | 0.024 s | **28.6 %** |

The generation itself is 14 ms of wall time; 200 episodes and 97 819 frames.

Two consequences. First, the plan's premise is wrong for this build: the sim,
the observation and the episode driver own about three quarters of the CPU, and
the network about a quarter. Second, the Item 2 gate is arithmetic: **a forward
pass that cost nothing would save 26 %, so a 25 % generation improvement needs a
batched pass that is essentially free.** A realistic 4–6× forward pass saves
19–20 %, below the gate, though above the plan's 10 % kill line.

**The batched pass was then built and measured against that gate, and killed by it.**
`Genome::forward_block` evaluates `BLOCK_LANES` (four) genomes interleaved — one accumulator chain
per lane, each in its own left-to-right order, so every lane is bit-for-bit the scalar pass —
and `tests/performance.rs` times it beside `Genome::forward` on the same population:

| shape | scalar | batched | speedup |
|---|---|---|---|
| 108 → 16 → 11, no memory | 1 015.0 ns | 892.9 ns | **1.14×** |
| the same with feedback 16 (the reference level's timed shape) | 1 195.0 ns | 1 241.8 ns | **0.96×** |
| 108 → 32 → 16 → 11 | 2 622.4 ns | 1 914.0 ns | 1.37× |

On the same run the forward pass was **15 % of a generation** (1 174 ns/decision, 24 503
decisions, 0.029 s of forward CPU against a 0.032 s generation on six cores). Better than a 1.14×
forward pass therefore buys **about 2 % of generation wall time**, and on the shape the reference
level actually uses — feedback 16 — the batched pass is *slower* than the scalar one, because each
lane's recurrent state has to be copied into and out of the block scratch and the two extra `Vec`s
there outweigh the interleaving.

**Item 2 fails its kill criterion (10 %) by a factor of five and is not routed into
`evaluate_population`.** The premise in the plan — "load the weight row once and apply it to every
genome" — is false for this population: genomes do not share weights, so there is no row to reuse,
and the only parallelism left is four independent scalar chains, which `dense` already keeps in
flight *inside* one genome (`DENSE_ROW_BLOCK`). `forward_block` stays in the tree as the measured
artifact with its bit-identity property test, and nothing calls it: the scalar path is what
population evaluation uses, because the measurement says the scalar path is faster on the shape
that matters. A later attempt should start from the 85 % the sim and the driver own, not from the
multiply count — the observation builds three `Vec`s per decision, and that is where the headroom
is.

## Item 1: jump-timing is neutral on its two levels, so far

`--jump-timing-share 0.5` against the default, pop 200, 300 generations,
24 seeds, `train-cli bench` (the arm files were written under a shared name and
the per-seed rows were lost when the second level overwrote the first; the
summary lines below are from the run log):

| level | default | `--jump-timing-share 0.5` |
|---|---|---|
| `l2-first-gap` | 24/24, median **2.0**, mean **2.0** | 24/24, median **2.0**, mean **1.9** |
| `l4-rhythm` | 24/24, median **8.0**, mean **7.8** | 24/24, median **8.0**, mean **8.2** |

The median does not move on either level, and the mean moves in opposite
directions by less than half a generation. The arm **stays off** (the shipped
default remains 0.0). A paired rerun with distinct arm names is still owed
before the result is quoted as a sign test; the direction is already clear
enough that no default should move on it.

**The plan bonus on its own level, closed at twelve seeds.** `--plan-bonus 50` against the default
on the `elements` protocol (pop 200, 2 000 generations, both arms complete):

| arm | solved | first win |
|---|---|---|
| default | **0 of 12** | — |
| `--plan-bonus 50` | **1 of 12** | seed 1, generation **863** |

The paired sign test is **p = 1.0** on the one discordant seed, so the arm has **not** moved the
plateau: one solve in twelve where the default has none is exactly the reading a single lucky
seed produces, and seeds 2–12 are indistinguishable (the bonus arm's best fitness on the unsolved
seeds is higher on seven of the eleven, lower on four — 1 341 against 1 136 on seed 11, 990 against
1 093 on seed 2). The
seed-1 solve reproduced bit-identically across two separate sweeps and across a rebuild, which
says the run is deterministic rather than that the solve is robust. The knob stays off; the honest
summary is that it is the only setting in this pass that has ever solved this level, and it does
so once in twelve.

## Item 4: lessons move gauntlet, CEM is a wash on tower

Both halves are off by default, and both have now been run on a 12-seed protocol.

**Mid-run lessons**, `--lesson-every 20` against the default on `gauntlet` (pop 200, 300
generations): **8 seeds won by lessons, 3 by the default, 1 tie** (exact sign test p = 0.23),
median first win **8 against 16** and mean 11.4 against 19.4. The direction is the one the plan
predicts and the sample cannot establish it; the plan's own gate is `l11-the-elements`, which is
still running.

**Plateau CEM**, `--plateau-window 20` against the default on `tower`: **2 wins each, 8 ties,
p = 1.0**, same solved count (9 of 12) and same median (53). It flipped seed 1 from unsolved to
solved at 249 generations and seed 6 from solved at 146 to unsolved, which is a wash with a
longer mean (83.4 against 78.3). No movement on the plateau it was written for.

## Item 13: growth does not move the tower

`--growth-units 4` against the default on the `tower` protocol (12 seeds, pop 200, 300
generations) is a wash: 2 seeds won by growth, 1 by the default, 9 ties, exact sign test
**p = 1.0**, and the growth arm **lost a solved seed** (seed 6: unsolved against the default's
146). The operator inserted units at the stall, and the extra capacity bought nothing on the
plateau it was grown for. The plan's kill criterion says grown capacity that does not move its
plateau was never the constraint there; the operator stays behind `growth_units = 0`, with this
reading recorded rather than the knob deleted, because the Item 13 gate names the resumed-l11
clock test rather than the tower.

## Item 3/12/13 groundwork: function-preserving input growth

`Genome::grown_to_inputs` and `NetShape::with_inputs` are in
`crates/evo/src/genome.rs`. They widen the observation by appending zero
columns to each first-layer row, **before the recurrent columns**: layer 0
reads `[observation; memory]`, so appending at the very end of the row would
move the old memory weights onto new observation cells. The tests grow both a
memoryless net and a feedback-16 net by six inputs and compare every output
and the recurrent context bit for bit over a 24-decision sequence. This is the
migration proof Items 3, 12 and 13 all need; it changes no default and no gene.

## Item 3: sensors v14 was built, measured, and removed from the tree

Sensors v14 was implemented in that pass — eight append-only columns (the nearest burning fuse's
countdown and position, the nearest threat's velocity, the second mover's velocity and phase),
`sensors::VERSION` 14, and a `ChampionFile::migrate_sensors` that grows a previous-layout file by
zero columns. It is **not in this tree**: `sensors::VERSION` is 13, and there is no branch, stash
or tag holding the attempt, so only the migration helpers (`NetShape::with_inputs`,
`Genome::grown_to_inputs`, `ChampionFile::migrate_sensors`) and
`crates/evo/tests/migration.rs` survive. It was not landed, for two independent reasons.

The first is the plan's own rule 1. Item 2 measured the forward pass at **26 %** of a generation
(see above), so a perfect batched pass would save 26 % and a realistic one 19–20 %, below the 25 %
bar that Item 3 is gated on. The input growth costs about 7 % per decision at the current shape;
spending it before Item 2 has banked more is exactly the trade the plan forbids.

The second is the gate the previous attempt died on, and it is worth recording because it is a
*test* finding rather than a sensor finding. At 108 inputs the paired warm-quarter canary passes.
At 116 it failed on `l5-tightrope`: warm 199 generations against random's 168, with all five seeds
solved. Neither gate level contains a walker, flyer, mover or crumble, so **all eight new columns
read zero on both**, and the clone's own policy is unchanged — its mean fitness over eight episodes
is the same to the hundredth at both widths (l2 260.80, l5 190.40). The flip is therefore the
documented re-roll of every random population from finding 63, not a worse warm start: over twenty
seeds the same gate reads warm 636 against random's 871 at 116, and 601 against 1 267 at 108, while
the per-seed median is a wash at both widths (22 against 21.5, and 15.5 against 16). Five paired
seeds of a heavy-tailed first-win total is a coin flip on this level, and it was passing at 108 on
that coin flip. The canary is left as it is rather than tuned to the new draw; what the reading
says is that **an append-only bump still re-rolls the gate**, because the trainer's mutants and
random remainder are drawn from a longer genome.

**Rebuilt a second time, and the failure reproduced.** The append-only half of v14 was written again
from scratch — eight columns at 108–115: the nearest burning fuse's countdown and tile
(`fuse_left, fuse_dx, fuse_dy`, read off `Sim`'s own fuse state through a new `Scene::fuses`), the
nearest threat's velocity in body-speeds (`threat_vx, threat_vy`), and the second-nearest mover's
velocity and cycle phase (`mover2_vx, mover2_vy, mover2_phase`). `sensors::VERSION` went to 14,
`N_INPUTS` to 116, and the existing `ChampionFile::migrate_sensors` / `Genome::grown_to_inputs`
pair carried a v13 champion across by zero-weight growth. The patch half was deliberately left out:
drawing `ENC_ENEMY` into the 9×9 grid changes the *values* of 81 inputs every champion already
reads, so a zero-weight growth is not policy-preserving on exactly the levels it is for, and the
migration would stop being a proof.

The canary failed with **the same numbers as the first attempt**: `l5-tightrope`, warm **199**
generations against random's **168**, with all five seeds solved on both arms. Different eight
columns, same width, same result. That rules out the explanation the previous pass recorded — that
the flip was the documented re-roll of every random population rather than the extra inputs. The
width is what moves the gate.

Raising the canary to twelve seeds — a stronger test, run before the revert to see whether five
seeds were simply a tail — failed on the other level for the same reason: `l2-first-gap`, warm
**38** generations against random's **33**, 12 of 12 solved on both. That is the level where random
is already fast, and it is exactly where the warm start's advantage lives; the two arms are paired
on the same 48 random genomes, so the deficit is not a different draw of the population.

**Why an append-only bump can still cost search.** Mutation is applied at
`mutation_rate × n_genes`, so widening does not spread a fixed budget thinner — per-gene pressure is
constant and the eight new columns are behaviourally dead on `l2` (no movers, enemies or crumbles).
What changes is *which genes each draw mutates*: the trainer's RNG stream is a function of genome
width, so every mutant in both arms is a different net. The warm arm's whole edge on a five-to-twenty
generation level is smaller than that shift. An append-only sensor bump is therefore **behaviourally
safe and search-unsafe**: it cannot change a policy, and it still changes what the search finds.

**Verdict: reverted, fourth time.** `sensors::VERSION` is 13 and the vector is 108. Three independent
reasons, none of them the eight columns' own merit — and the third is the one that should have been
checked first: **the battery gate's premise is false**, because both levels it names already solve
at generation 0 with the v13 default (`bat-crumble-bridge` 12/12, `bat-double-threat` 12/12, see
**The gates, run**), so no perception change can move them. The other two: the plan's rule 1 is unmet (`Item 2` banked about 2 %, not
the 15 % it required), and the canary the plan names as the stop condition reproduces. A future
attempt needs a decision about the *test*, not the sensor layout: either pair the arms on the gene
stream so both draw mutations for the same 108 positions, or accept the canary as the decision
procedure and pay its cost in generations — and the battery gate still needs the pack retrain,
which is machine time no code change substitutes for.

What survives is the machinery the plan needs whichever bump lands first: `NetShape::with_inputs`,
`Genome::grown_to_inputs` (zeros inserted **before** the recurrent columns, because layer 0 reads
`[observation; memory]`), `Genome::grown_with_commitment`, and `ChampionFile::migrate_sensors`,
which refuses anything but the immediately previous layout. `crates/evo/tests/migration.rs` proves
the artifact chain on the real saved l11 champion (feedback 16): grown by the eight columns a next
bump would append and fed the old cells followed by zeros, it replays to the same frame count,
outcome, best x and progress, bit for bit. The sensor layout in this tree is v13/108, and the l11
champion in `champions/` is current against it.

## Item 8: there is no narrow phase draw to widen

The item's premise is a training draw that is narrower than the phase space the validators sample.
This build does not have one, in either half:

* Training episodes are a function of `(master_seed, generation, trial)` under pairing, or
  `(master_seed, generation, genome id, trial)` without it, and the sim draws the patrol's spawn
  offset and direction and a flyer's position uniformly over the space a level permits. The
  generation is part of the seed, so the world is re-rolled every generation and no phase can be
  memorised. There is no range constant to widen.
* Validation is already a fixed held-out set: `validation_seed` uses the `u32::MAX` generation
  sentinel and a genome id no issued genome has, so two records meet the same twelve worlds
  (`GaConfig::validation_episodes`), whatever the episode policy. It is compared on solve count
  first, then mean, and `--stop-on-validated` already reports it in `train-cli bench`.

So Item 8 has no implementation in this codebase and no default moves. The mechanism it was
written to add — a hard practice draw, an honest exam, and a first-class gap — is already the
shape of `common_episodes` plus `Validation`. What is genuinely missing is the *gap* itself: no
run, champion or difficulty row computes a training-phase against held-out-phase reading, so the
plan's step 3 is the one part of Item 8 that has no code at all. It is a reporting change with no
training effect, and it is left for a pass that can re-take the table.

## Item 10: the committee, its artifact and its gate

`evo::Committee` runs one forward pass per member on the same observation, takes
the majority action, and breaks a tie by the earliest member that voted for a
tied action. Each member has its own `ActScratch`, so recurrent members keep
their own memory; `Committee::validate` plays the vote and its best member on
the same fixed seeds, and `CommitteeReport::passes` is the plan's rule (never
worse than the best member on completion; equality passes). `showcase` turns the
vote into a decision stream a viewer can replay.

What was missing was the artifact, not the vote: the splash screen's own docs
recorded that a committee assembled from whatever genomes happen to be in memory
would be a different committee on every start. `CommitteeFile` is that artifact
— format, level hash, sensor version, cadence, the run's master seed and its
validation count, and the members in leaderboard order — with `save`/`load`,
`check_compat` and a `validate` that plays the run's own fixed validation
episodes. `train --committee <k>` writes one beside `champions/` and prints the
gate's two counts; `train-cli committee --file ... --level ...` re-runs the gate
from the file alone and exits non-zero if the vote lost. The file round trip and
the cross-level refusal are pinned in `committee.rs`.

The title screen now loads it: `Splash::open` prefers `committees/<level>.committee.json` when
it validates, plays the vote through `CommitteeRun` on the file's cadence, and captions it with
the gate's counts; a file that fails the gate, is missing, or is from another build falls
through to the single champion and then to the scripted demo. The pack-wide gate (the committee
against its best member across every level) is still a machine run.

## Item 5: the early cut, and the scope of its guarantee

`--cut-stuck <ticks>` ends an episode once the sim's own no-progress counter
reaches that many frames and scores the part that was played. `0` is the
default, so every run before the knob is bit-identical. Every term in `score` is
monotone or one-shot, so the partial episode's score is the score of the ticks
that happened; a body that could still escape would have scored more by playing
on, and that is stated rather than hidden.

The plan's hard rule is pinned on the case where it is a theorem: a genome of
zeros walks into a wall and stands there, and the cut episode must score
bit-identically to the played-out one while costing fewer frames
(`the_no_progress_cut_scores_a_wall_stander_exactly_as_the_full_episode_does`).
On a real level the cut does change the search, because a body that idles for
the window and then escapes is cut before the escape: on `l9-wait-and-climb`,
pop 200, seed 2, 20 generations, 120 idle frames raised the pass rate from
**17 741 to 20 867 population-episodes per second (+18 %)** and changed the
config's best genome. So the honest claim is a cheaper evaluation at the same
episode count, with the champion-equality half of the gate failing on levels
where late escapes matter. The knob stays off until an equal-episode arm says
the cheaper reading is worth it.

## Item 7: a recorded human run seeds a run

`--from-demo <file>` loads a `HumanDemo` written by the playtest, checks its
level hash, physics and sensor versions, cadence and that it reached the flag,
fits it with the ordinary imitation code (no planner in the loop, on purpose),
and seeds `pop / 4` slots like `--from-route`. Verified end to end on
`l2-first-gap`: 139 decisions, 300 epochs, **95.0 % agreement**, two of eight
slots seeded. The gate — a demonstration against the same run without one on a
level where the default stalls — has not been run.

## Item 11: the technique archive breeds

`--technique-children <n>` draws a share of each generation's parents from the
technique archive's cells instead of the leaderboard, with the same fine step
the behaviour archive's children use (an incumbent is the best at one technique,
and a coarse move destroys it). It is selection only: no fitness changes, no
version moves, `0` by default, and a run reports the children spent beside the
cells they came from. Pinned by
`technique_children_are_bred_from_cells_and_off_is_off` on a pit level. The
The 24-seed l11/battery arm has not been run; the cheap proxy on `gauntlet` (12 seeds) reads
**5 wins for the share, 7 for the default, p = 0.77**, median 17.5 against 16 — no gain there, and
no harm outside the noise either. A mechanism needs its own level, which is why the plan names
l11 and a technique-gated battery level rather than the pack's third level.

## Item 1: islands lose to one population of the same total size

`train-cli islands --islands 4 --pop 50` against the default on the `tower` protocol (12 seeds,
300 generations, stop at first win, migration every 25 generations): **9 seeds won by the default,
1 by the islands, 2 ties**, exact sign test **p = 0.0215**, solved **9 of 12 against 5 of 12**,
median first win **53 against 134**. The one seed islands gained (1) is one the default never
solved; on all four seeds both solved (4, 8, 9, 11) the default was faster (38/120/53/64 against
134/130/92/237), and islands never solved the five seeds the default did (2, 3, 6, 7, 12). The
two ties are the two seeds neither arm solved (5, 10).

The comparison is equal-budget in genomes an episode — 4 × 50 is the default's 200 — and that is
also its confound: 50 is a quarter of the population size every other measurement in this file
uses, and a 50-genome island has eight immigrants arriving into it per stall rather than eight
into 200. So the reading says *this* islands configuration is a loss on a plateau, not that a ring
of full-size populations would be one. A 4 × 200 arm is four times the budget and a different
experiment.

Two process notes. `islands` is its own command and does not write a bench arm, so the arm file
this was compared with was assembled from the command's own log (per-seed first win, and the
largest `best island maximum` each seed printed); the counts and the sign test above are read from
it, and that derivation is the reason the row is not a `bench` file like the others. And the
islands arm shows the plateau's own shape: best island maximum sat at 594 for 200 generations on
seed 1 before one island broke through at 285.

## WP8: snapshot probes are a wash on gauntlet

`--snapshot-every 20 --snapshot-probes 4` against the default on the `gauntlet` protocol (12
seeds, pop 200, 300 generations): **5 wins each, 2 ties, exact sign test p = 1.0**, solved 12 of
12 in both, median first win **15.5 against 16.0** and mean 18.0 against 19.3. The probes are a
small net gain in the medians and no gain at all in the paired direction.

The probe batch is four extra genomes every 20 generations — 60 probe episodes against the
protocol's 105 000 (200 genomes × 300 generations × the 1.75 episodes a raced walker level spends)
— so this is an equal-episode comparison to within a rounding error, and the wash is a wash rather
than a budget artefact.

What `gauntlet` cannot say is whether the mechanism works where it was written for, since it has
neither a deceptive pit nor a backtrack. That gate was then run on `bat-pit-bait` — see the next
section, where the archive **loses**. `bat-backtrack` at 24 seeds and the stall-chain re-run are
still unrun.

## WP8 on its own level: the archive solves fewer seeds than the default

The plan's gate for the snapshot archive is a deceptive battery level, and `bat-pit-bait` is one.
The `tower` protocol's 300 generations solves **nothing** on this level (0 of 2 seeds in the probe,
best fitness ~180 and stuck), so all three arms below ran **600 generations**, 12 seeds, pop 200,
with `--level levels/bat-pit-bait.json`; the arm files record that override.

| arm | solved | median first win | mean |
|---|---|---|---|
| default | **5 of 12** | 285 | 261.8 |
| `--snapshot-every 20 --snapshot-probes 4` | **2 of 12** | 352 | 352.0 |
| `--plan-bonus 50` | **3 of 12** | 307 | 231.0 |

The paired sign test against the default is **4 win / 1 loss, p = 0.375** for the archive and **4
win / 3 loss, p = 1.0** for the plan bonus. Neither is a win in direction, and the archive — the
mechanism this level was chosen for — **loses three of the five seeds the default solves** (3, 5,
10), gains one (2), and is slower on the fourth both solve (565 against 134); it leaves **ten**
seeds unsolved where the default leaves seven. The one seed it wins by a large margin is seed 2
(139 against the default's 455), which is the shape of a lucky restart rather than a mapped level.

That is a stronger negative than the `gauntlet` wash, and it is not a budget artefact: 30 probe
batches of four are 120 episodes against 120 000 genome-episodes (0.1 %), so the arm spent
essentially the same episode count for half the solves. The archive's restarts are therefore not
finding the pit's far side at this probe rate. Whether that is the mechanism or 120 probes being
too few to map a 36 × 16 level is the open question, and answering it means either a denser probe
rate as its own arm or a trainer change that pays for probes out of the generation's episodes —
both measurable, neither run.

**The plan bonus on the same level** is the second reading for that knob and it agrees with the
first: 3 solves against 5, p = 1.0, median 307 against 285 — no gain, and its fastest solve (seed
11, generation 70) is cancelled by seeds 2, 5, 10 and 12 going the other way.

## Item 9: the fusion code exists and the CLI cannot reach it

Auditing SMARTER_AI_PLAN.md against the tree, rather than against these notes, turned up a claim
in the Item 9 row that was wrong: fusion is **not** unbuilt. `crates/evo/src/curriculum.rs` has
`fuse_specialists` (neighbour legs crossed with the shipped uniform crossover, `FuseConfig`
defaulting to 8 children at `cross_rate` 1.0) and `fused_seeding` (fused children first, then
the caller's whole-level genomes, deduplicated by exact gene equality), and
`the_fused_seeding_order_is_deterministic_and_deduplicated` pins the order. `segment_campaign`
keeps one clone per leg in `SegmentCampaign` and writes each leg's population to
`<stem>.segment-NN.population.json`, so the parents Item 9 names do reach disk.

What is missing is the wire: `train-cli campaign` never calls `fused_seeding`, there is no
`--fuse` share on any command, and the campaign's written artifact is one whole-level champion.
So Item 9 is in exactly the state Item 6's flags were in before this pass — a reviewed mechanism
with a test, unreachable from the command line — and its gate (fused seeding against the
campaign's continuation, episodes to validated win on `l11` and `l10`) is unrun.

## Item 6: seeding from another run's population is the first large win in this pass

`--generalist <population.json> --generalist-share 0.25` against the default on the `tightrope`
protocol (12 seeds, pop 200, 300 generations, stop at first win): **12 seeds won by generalist,
0 by the default, 0 ties**, exact sign test **p = 0.0005**, median first win **2.0 against 8.0**
and mean 1.9 against 8.7. Every one of the twelve seeds moved, ten of them by more than three
generations, and the generalist's slowest seed (4 generations, seeds 2 and 11) is still only as
slow as the default's fastest (4, seed 3).

Two caveats are part of the reading rather than after it. The checkpoint is **not** the pack-wide
generalist this item is written for: it is the final population of a 20-generation pop-64 run on
`l2-first-gap`, the pack's *easiest* level, so part of the gain is a warm population rather than a
*general* one, and `bench` prints the level-hash mismatch every time it is used. And the arm is
capped at 25 % of generation zero, which is the plan's own clamp (`--from-route`'s earlier
measurement says warming every slot is worse). What this establishes is that the warm-start
plumbing is decisive on this level and cheap to use; what it does not establish is that a
cross-pack checkpoint beats a same-level one, which needs `--save-population` from a suite run.

Two defects were found on the way. `--generalist` and `--generalist-share` were parsed but missing
from `TRAIN_OPTS`, so the documented arm was **unreachable from the CLI** — a run that passed them
was refused as unknown. And a seeded `bench` arm still wrote `start: noise`: the arm file named
only `--from-champion` and `--resume`, so a generalist, route, plan or demonstration seed produced
a record indistinguishable from a cold one. `bench::start_of` now names all five seed sources and
appends the generalist path and share, pinned by `an_arm_names_the_population_it_started_from`.
The generalist arm was then **re-run after the fix and reproduced every per-seed row exactly**
(same 12 wins, same 2.0 median, same 1.9 mean), so the reading above is the recorded one and not
a log nobody can rebuild.

## The four items this pass finished: 5, 6, 8 and 9

The plan's remaining implementation work was Item 5's two halves, Item 6's rotation and refresh,
Item 8's gap reading and Item 9's wire. All four are now in the tree with tests.

**Item 5, step 1 (the band).** `--cut-stuck` used to cut on the stuck timer alone. It now keeps a
cut only when the genome sits strictly below the selection cut (`--cut-margin`, default 0; a
positive margin replays anything within that much score uncut). On `l9-wait-and-climb`, pop 200,
20 generations, seed 2, debug build: **1 301 of 4 000 episodes cut, 1 845 population-episodes/s
against 1 605 with the cut off (+15 %)**. The number is smaller than the old unconditional arm's
+18 % because the band replays the cuts that could have mattered — which is the point. The speed is
real and the 12-seed `chain` gate (see **The gates, run**) shows what it costs: the arm solves two
fewer seeds than the default, so the speed is not worth the solves.

A finding falls straight out of the default: at a *win's* margin (**10 200**, the largest one-shot
jump the score can make) **no episode on the plateau can be cut at all** — every genome sits
within a win of the cut, every cut is replayed, and the run is 40 % slower than no cut (0 of 4 000
cut, 1 021/s against 1 431/s). The plan's condition taken at its most conservative reading
therefore deletes the saving it exists to collect, which is why the shipped default is the exact
band and not a win.

**Item 5, step 2 (the refund).** `--cut-refund` turns the ticks the cut did not play into whole
episodes at the generation's mean episode length and spends them as one extra trial each on the
genomes in the first quarter of ranks below the cut, in **one parallel pass** (a per-genome pass
made the refund cost several times what the cut saved, and was measured before being replaced).
The arm does not pay on `l9`: **1 478/s against 1 605/s with the cut off (-8 %)**, 1 327 episodes
cut, 96 genomes re-read in a 6-generation probe — the refund's extra episodes cost more than the
cut frames save at this level and population. Implemented, measured, and off.

**Item 6, step 1 and 3.** `train-cli generalist --levels a,b,c` keeps one population trained across
the pack in rotation, with the cursor derived from the checkpoint's own generation so a second
invocation continues the cycle; `--refresh <champions dir>` puts the pack's own champions into the
back slots, so the transfer runs both ways. Verified end to end: generation 4 → 6 across two
invocations, a refresh at generation 6 that trains nothing, and a run seeded from the result
(`seeded 8 generalist elite(s) into generation zero`). No paired arm yet; the mechanism the plan
asked for exists.

**Item 8, step 3.** A champion now carries its own `phase_gap` — training-draw wins and the
held-out validation wins — and `train` prints `gap training 3/3 held-out 10/12 gap +0.17` beside
the verdict it qualifies. Measured on a real run (`l3-gauntlet`, 3 generations): `train_wins 0/3`,
`held_wins 0/12`, written into the file and preserved across a save/load. A suite is skipped on
purpose: its validation set is one level's.

**Item 9.** `campaign --fuse <N>` (and `--fuse-cross-rate`) breeds N crosses of the campaign's leg
specialists with the shipped crossover and seeds them into generation zero before the whole-level
phase. Verified: 4 legs → 4 crosses seeded, and a one-leg campaign refuses with "a cross needs two
parents" rather than seeding a copy of one specialist. `Trainer::seed_donors` is the shared
primitive under both this and the generalist share.

## The gates, run

Every item that has a gate has now been run on the repository's own protocol harness
(`train-cli bench`, release build, 12 seeds, pop 200), not estimated from a single arm. The arms
are recorded under `bench/`.

**Item 5, the cut — fails the gate.** `chain` (`l9-wait-and-climb`, 12 seeds, 400 generations, stop
at first win):

| arm | solved | median | vs default |
|---|---|---|---|
| default | **11 of 12** | 100 | — |
| `--cut-stuck 60` | 9 of 12 | 162 | 7–4, p = 0.55 |
| `--cut-stuck 60 --cut-margin 10200` | 6 of 12 | 91.5 | 7–4, p = 0.55 |
| `--cut-stuck 60 --cut-refund` | 9 of 12 | **87** | 5–7, p = 0.77 |

The arm loses solves in every configuration and never wins the sign test: the band turns a 400-
generation sweep into an 11-of-12 sweep into a 9-of-12 one and a 6-of-12 one, and the refund buys
a better median while still solving two fewer seeds. Its speed is real (the `l9` episode count
above) and it is not free: the episodes it declines to play are the ones a late-generation
mutation would have needed. **Off is the default and stays the default.**

**Item 6, the generalist — passes on transfer, and the difference between the two readings matters.**
A 30-generation generalist rotated over `l2-first-gap` and `l3-gauntlet` only — **`l5-tightrope`
held out** — seeded at a quarter share, then swept on `tightrope`, 12 seeds, 300 generations:

| arm | solved | median | mean | vs default |
|---|---|---|---|---|
| default | 12 of 12 | 8.0 | 8.7 | — |
| generalist, **held out** from its rotation | 12 of 12 | **4.0** | 4.3 | **10–1, p = 0.0117** |
| generalist trained on tightrope too (retention, not transfer) | 12 of 12 | **0.0** | 0.0 | 12–0, p = 0.0005 |

The middle row is the gate. The third row is the same checkpoint protocol with the graded level in
its own rotation, and it is only retention: a population trained on a level trivially "starts"
solving it, which says nothing about the item's claim. The held-out row says a generalist built from
two *other* pack levels halves the generations to a solve on a level it has never trained on, and
wins ten of the eleven discordant seeds.

**Item 9, fused seeding — mechanism confirmed, effect unproven.** `campaign` on `l5-tightrope`
(pop 64, 8 generations per leg, 20 on the whole level), `--fuse 4` against `--fuse 0`, 12 paired
seeds: **11 of 12 solved against 10 of 12**, the one disagreement seed 7 (fused wins). A sign test
over one discordant pair is p = 1.0, so the campaign effect is not established; what is established
is that four crosses of five leg specialists enter generation zero and the whole-level phase then
solves one seed more with a higher best fitness (11 388.8 against 11 378.3, 444 frames against 499).

**Item 3, the battery — the gate's premise does not hold.** `train-cli battery` passes all seven
`bat-*` levels (reach and plan), and the two the item names for its gate train to a solve under the
**v13 default** at the `hazard-floor` protocol shape: `bat-crumble-bridge` **12 of 12 at
generation 0**, `bat-double-threat` **12 of 12**, median 3.5. The item exists to move those two from
unsolved-by-default to solved, and they are already solved at the first generation, so the gate
cannot distinguish a policy that reads a fuse from one that does not. That is a finding about the
battery, not about v14: the levels are a planner test (4 and 5 expansions), too short for a
perception change to show up.

**Item 2** has no arm to run: the gate is generation wall time, and the forward pass is 15 % of a
generation with the batched pass slower than the scalar one on the reference shape, so the ceiling
is ~2 % against a 25 % bar.

## Where every item stands

| item | code | measured |
|---|---|---|
| 1 jump-timing | shipped, off | neutral on l2/l4; stays off |
| 1 plan bonus | shipped, off | elements: 1 solve in 12 (seed 1, gen 863) against the default's 0, p = 1.0; bat-pit-bait: 3 solves against 5, p = 1.0 — no movement on either |
| 1 islands | shipped, off | tower, 4 × 50: 1 win / 9 losses, p = 0.02, median 134 vs 53 — a loss at equal genome budget |
| 2 batched pass | built, measured, **killed**: `Genome::forward_block` is not routed into evaluation | 1.14× batched on the memoryless shape, **0.96×** on the reference level's feedback shape, against a 10 % kill line; forward pass is 15 % of a generation on this machine |
| 3 sensors v14 | **not in the tree**, rebuilt once more and reverted; `sensors::VERSION` stays 13 at 108 inputs | **the gate's premise is false**: both levels it names are already solved by the v13 default (`bat-crumble-bridge` 12/12 at generation 0, `bat-double-threat` 12/12, median 3.5), so no sensor can move them; the warm canary also reproduces at the same width (l5 199 vs 168, l2 38 vs 33 at 12 seeds) |
| 4 lessons + CEM | shipped, both off | gauntlet: lessons 8 wins / 3 losses, p = 0.23 (median 8 vs 16); tower CEM: 2/2, p = 1.0; l11 gates outstanding |
| 5 early cut | shipped, off, with the band and the refund | **12-seed `chain` gate: fails on solves** — default 11/12, band 9/12, win-margin 6/12, refund 9/12; directions are 7–4 and 5–7, p = 0.55 / 0.77 |
| WP8 snapshot probes | shipped, off | gauntlet: 5 wins / 5 losses, p = 1.0, median 15.5 vs 16; bat-pit-bait (600 gens): 2 solves against 5, 1 win / 4 losses, p = 0.375 — a loss on its own gate level, at 0.1 % more episodes |
| 6 generalist seeding | shipped, off; `generalist` command keeps one population rotating across the pack and refreshes it from pack champions | **12-seed `tightrope` gate, level held out of the rotation: passes** — 12/12 solved at median 4.0 against the default's median 8.0, 10 wins / 1 loss, p = 0.0117 |
| 7 human demo | shipped, on demand | 95 % clone agreement; stall gate outstanding |
| 8 phase draw | no lever exists; the **gap reading** is now in the champion file and the train summary | measured 0/3 training vs 0/12 held-out on gauntlet |
| 9 fused seeding | `campaign --fuse <N>` / `--fuse-cross-rate`, on the shared `Trainer::seed_donors` primitive | **12-seed campaign on `tightrope`: 11/12 solved against 10/12**, one discordant seed (7), p = 1.0 — mechanism confirmed, effect unproven |
| 10 committee | shipped + file + gate command + title-screen display | pack gate outstanding |
| 11 technique share | shipped, off | gauntlet: 5 wins / 7 losses, p = 0.77, median 17.5 vs 16; l11/battery gate outstanding |
| 12 cadence head | shipped, off | gauntlet proxy: 7 wins / 5 losses, p = 0.77, median 12 vs 16; the plan's v14 arm-vs-arm gate is blocked with Item 3 |
| 13 hidden growth | shipped, off | tower arm: 2 wins / 1 loss / 9 ties, p = 1.0, and one solved seed lost — no movement |


## Review fixes, 2026-10-19: the pass, audited

An outside review of this pass found five things to fix. All five are now fixed in the tree;
nothing here moves a default or a gate reading above.

**The migration proof only passed on one machine.** `crates/evo/tests/migration.rs` read the
author's own saved l11 champion out of `champions/`, which is gitignored, so the three tests
failed on every clean checkout — CI on `main` was red for exactly this while the rest of the
suite (212 evo, 269 app) was green. The tests now read a committed fixture
(`crates/evo/tests/fixtures/l11-the-elements.champion.json`): a feedback-16 genome whose
episode on l11 (1 567 frames, stuck) was run through the ordinary runner and wrapped by the
ordinary `ChampionFile::new`, regenerated by `cargo run -p evo --example
make_migration_fixture`. Same proof, portable evidence.

**A cut run could crown a champion it could not reproduce.** Replay always evaluates uncut,
but a run trained with `--cut-stuck` records fitness from cut episodes, and nothing stopped
that run writing a champion file whose numbers replay would contradict. `TrainingSummary`
now records `cut_stuck_frames`, and `ChampionFile::save` refuses a summary that has it set,
naming the knob and the remedy. Measurement with the cut stays available (bench arms record
themselves); crowning requires an uncut run.

**Item 2's killed code is removed, not kept.** `Genome::forward_block`, `BlockScratch`,
`BLOCK_LANES` and their bit-identity tests are gone (roughly 350 lines), along with the
batched half of the performance probe. The kill criterion said revert; a measured artifact
with no callers is still dead code, and the readings that condemned it are the ones recorded
under Item 2 above. The probes that remain measure the scalar pass's share and what ends an
episode, which are the numbers a future performance item needs.

**Item 4's lessons: a clarification the review got half wrong.** The review claimed the
lessons are not DAgger because no visited states are labelled. In fact `imitate::from_plan`
runs up to six DAgger rounds: the student clone is rolled out on the real sim and refit on
the states it actually reaches. What differs from the plan's letter is the labelling and the
student: labels come from the plan's action at the *nearest demonstrated state* (a
trajectory expert, chosen in `imitate.rs` because the plan is open-loop and cannot be
re-queried from an arbitrary state), and the states rolled out are the lesson clone's, not
the record genome's. The mechanism is DAgger with a nearest-neighbour expert; the plan's
fresh-plan-per-visited-state variant remains unbuilt, and the gauntlet reading above belongs
to the shipped one.

**Bench arms can no longer overwrite another level's rows.** The jump-timing arm lost its
per-seed data when two levels were recorded under one arm name. `write_arm` now reads an
existing arm back and refuses when its level hash differs from the run about to replace it;
a same-level re-run, the reproduce-a-reading workflow the generalist arm used, still writes.
Pinned by `an_arm_for_a_different_level_cannot_overwrite_an_existing_arm`.

**The observation no longer allocates on the hot path.** Item 2's redirection said the
headroom is in observation and the driver, not the forward pass: building a scene allocated
three vectors per decision (mover rectangles, velocities, phases). `Sim::observation_into`
builds them in caller-owned `ObservationScratch` buffers, and the episode runner, both
committee loops, the imitation rollout and both route-play drivers use it. Values are
unchanged by construction and by pin: a sim test compares the two forms bit for bit over an
episode on l11 (which has a mover), and every champion/replay determinism test in the suite
stands behind it. No before/after timing is quoted: the readings that would make the claim
belong on the reference machine, and this change was made because the allocations were
there, not because a percentage was promised.

## Item 14: repair children — a mistake the run can name

The complaint this starts from is not a number: a run reaches the flag and then stops getting
better, because nothing in the objective says *which decision* was wrong. Fitness is whole-episode
(progress, the win bonus, the speed bonus paid on a win, and the clock under mastery), so a policy
that dies at the same flyer two hundred times and one that dies there once and dawdles look alike to
selection. Every operator in `ga` asks "what if this genome were slightly different"; none asks
"what did it do wrong".

`evo::mistakes` adds the second question. Per generation, the best genome replays its own
representative episode with the last eight decisions captured (`EpisodeRunner::capture_decisions`),
every one of the eleven actions is held for 32 ticks from those states in a restarted snapshot, and
the earliest decision where an alternative **survives** and the chosen action does not becomes a
mistake. `learn` then moves that action's output row toward the hidden activation it saw and the
losing row away from it, by the smallest step that actually flips the argmax at that state. The
child is injected into the next generation and judged on a full episode like any other genome: no
fitness term, no champion, no recorded episode is touched.

**The first version was wrong, and the arm said so.** It also accepted a plain progress gain — "this
sprint got four pixels further before the same death" — and eight paired seeds on the custom level
came back 5 of 8 solved either way, median first win 131 generations with repairs against 101
without. A 32-tick rollout that prefers one dying action to another is describing the arc of the
fall, and the nudge then teaches an action at a state nothing could have saved. Survivors only, and
the earliest such decision rather than the last, is the shipped rule.

**Paired arm, 12 seeds, one level.** `train --level levels/custom-l1-flat-run.json --generations 200
--stop-on-win`, seeds 1–12, measured against `--repair-children 3` on the same seeds. First-win
generation, `—` for a seed still unsolved at 200:

| seed | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| off | 135 | — | 73 | 127 | — | 101 | — | 60 | — | — | — | 175 |
| **on** | 180 | **83** | 78 | 127 | — | **47** | **85** | 119 | **59** | **59** | **144** | **157** |

- **Solved: 6 of 12 without repairs, 11 of 12 with them.** The five seeds that only solve with
  repairs are the whole effect: 5 discordant pairs toward the arm, none against, exact McNemar
  p = 0.0625 (one-sided 0.031).
- **Among seeds that solve either way it does not speed anything up.** Six such pairs: 3 to the
  plain run, 2 to repairs, 1 tie; sign p = 1.0. The mechanism buys *arrival*, not tempo — which is
  the honest reading, because it only fires on episodes that lost.
- **Cost: about 3 ms on a 15 ms generation** (mean 0.015 → 0.018 s/gen at pop 200 on this machine),
  one replay plus 11 × 8 short rollouts.
- **Not the 24-seed gate.** One level, one machine, twelve seeds. `GaConfig::repair_children` is 0 by
  default and `--repair-children` is the arm; the window runs 3 because a watched run is where the
  mistake is legible. A 24-seed gate across the pack is still owed before this becomes a default.

## Phase 0: learning telemetry and a certified practice suite

The complaint this phase answers is not "the score is low". It is that a run can look identical
whether it is learning or scouting: a maximum that will not move while every agent dies at the same
ledge, a population that converges on one action and calls it a policy. Fitness is a whole-episode
scalar and cannot distinguish those. So Phase 0 builds the two things every later phase gates on:
a **reading of where the population actually got**, and a **fixed set of problems whose answer is
already proved**.

### What was added

* **`evo::telemetry`** — one `Telemetry` record per generation, computed from the results the
  trainer already has: `solved`/`wins`, the winning clock (best and median frames), where the best
  genome ended (leg, frame, decision, and whether it died), the per-leg census of every episode
  (`ended_legs`, `died_legs`, `reached_legs`), and the population's first-action histogram and
  entropy at a fixed spawn probe. Reporting only: no fitness term, no hash, no RNG draw. The one
  cost is 200 forward passes at spawn, far below a generation.
* **A leg is a route crossing**, not a checkpoint: `route_hops_closed`, the sim's own geodesic
  count. On a level with no route (`legs == 0`) every episode is in leg 0 and the record says so.
  That is the unit Phase 1's route compiler will use, so the two cannot disagree about "leg 3".
* **Surfaced three ways**: `train` prints `solved`, `clock` and the best genome's ending per
  generation; `--telemetry <PATH>` writes one JSON object per generation (`watch` accepts it too);
  and the window's Details panel has a "What it learned" section with the solved rate, clock, best
  genome's leg/frame/decision and one row per leg anybody ended on.
* **`sim::practice`** — a deterministic generator for twenty-four practice levels: six archetypes
  (`gap`, `spike`, `step`, `crumble`, `mover`, `mixed`) at four tiers. Each candidate is certified
  by the reachability oracle **and** the timed planner before it is kept; failing candidates fall
  back to a gap chain, so a level is never written uncertified. The suite is committed under
  `levels/practice/` and `train-cli fuzz --out levels/practice` regenerates it byte for byte.
* **`train-cli fuzz`** — prints the certification table (oracle, plan, frames, blocks, and the
  oracle and planner cost measured on a fresh parse of each file). All 24 pass: oracle 500 ms
  total in release, timed planner 5.92 s total, worst level 1.32 s (`p23-mover-t3`).
* **`tools/phase0_baseline.py`** — the gate below, re-runnable. It runs both arms on the pack, the
  battery and the suite, reads each run's JSONL, and prints the table.

### The gate, as run

Protocol: pop 200, **one episode per genome** (`--no-auto-trials --trials 1`, so the episode cost
is the same on a patrol level and a deterministic one), at most 100 generations, `--stop-on-win`,
curriculum off, seeds 1 and 2. The **random** arm is the same population on the same level for one
generation (200 episodes, seed 99) — the floor an unevolved population sets. Plan ms is today's
timed planner on a fresh parse: depth 48 for the generated suite (its certification budget), the
shipped default depth for the pack and battery. `D` = default, `R` = random. Clocks are
seconds; legs are the share of the final generation that crossed each of the first three route
legs; entropy is the first-action probe in bits (the maximum is 3.46).

| level | D solved | D first win | D clock best/med | D legs 0/1/2 % | D entropy 0→end | R solved | R legs 0/1/2 % | R entropy | plan ms |
|---|---|---|---|---|---|---|---|---|---|
| **pack** (12 levels) | | | | | | | | | |
| l1-flat-run | 2/2 | g0 / 200 ep | 6.15s / 8.40s | — | 3.43→3.43 | yes | — | 3.42 | 50 |
| l10-the-hunt | 2/2 | g5 / 1200 ep | 7.00s / 7.00s | 10/0/0 | 3.43→3.34 | no | 2/0/0 | 3.43 | 290 |
| l11-the-elements | 0/2 | — | — / — | 68/35/18 | 3.44→0.14 | no | 1/0/0 | 3.44 | 2530 |
| l2-first-gap | 2/2 | g1 / 400 ep | 12.77s / 20.95s | 10/1/1 | 3.42→3.31 | no | 2/1/0 | 3.42 | 80 |
| l3-gauntlet | 2/2 | g15 / 3200 ep | 11.07s / 12.98s | 24/8/1 | 3.42→2.09 | no | 5/0/0 | 3.42 | 170 |
| l4-rhythm | 2/2 | g5 / 1200 ep | 7.35s / 8.33s | 42/22/16 | 3.43→2.03 | no | 2/0/0 | 3.41 | 120 |
| l5-tightrope | 2/2 | g5 / 1200 ep | 7.48s / 25.78s | 10/7/6 | 3.43→2.61 | no | 0/0/0 | 3.42 | 250 |
| l6-marathon | 2/2 | g7 / 1600 ep | 9.38s / 9.53s | 16/13/5 | 3.43→2.47 | no | 0/0/0 | 3.42 | 580 |
| l7-ascent | 2/2 | g7 / 1600 ep | 7.22s / 14.25s | 8/6 | 3.45→2.76 | no | 0/0 | 3.39 | 430 |
| l7-wall-school | 2/2 | g4 / 1000 ep | 3.50s / 4.18s | 0 | 3.44→2.88 | no | 0 | 3.41 | 440 |
| l8-long-jump | 2/2 | g4 / 1000 ep | 6.27s / 6.32s | 12/0 | 3.43→2.66 | no | 0/0 | 3.42 | 100 |
| l9-wait-and-climb | 1/2 | g29 / 6000 ep | 18.20s / 18.20s | 49/24/13 | 3.44→1.60 | no | 2/0/0 | 3.38 | 1870 |
| **battery** (7 levels) | | | | | | | | | |
| bat-backtrack | 2/2 | g0 / 200 ep | 3.97s / 4.10s | 3/0 | 3.43→3.43 | yes | 4/0 | 3.40 | 10 |
| bat-crumble-bridge | 2/2 | g0 / 200 ep | 4.48s / 6.57s | — | 3.44→3.44 | yes | — | 3.45 | 30 |
| bat-deck-phase | 2/2 | g16 / 3400 ep | 8.98s / 10.70s | 8/8/6 | 3.44→1.89 | no | 0/0/0 | 3.45 | 120 |
| bat-double-threat | 2/2 | g2 / 600 ep | 4.85s / 4.98s | 0 | 3.43→3.34 | no | 0 | 3.42 | 50 |
| bat-patrol-wait | 2/2 | g1 / 400 ep | 4.95s / 5.68s | 2 | 3.44→3.33 | yes | 1 | 3.41 | 40 |
| bat-pit-bait | 0/2 | — | — / — | 17/6/6 | 3.41→2.23 | no | 2/0/0 | 3.41 | 970 |
| bat-spring-chain | 2/2 | g0 / 200 ep | 6.15s / 7.12s | 2/1 | 3.44→3.44 | yes | 1/0 | 3.40 | 30 |
| **generated** (24 levels) | | | | | | | | | |
| p01-gap-t0 | 2/2 | g0 / 200 ep | 7.08s / 14.42s | 4/0 | 3.44→3.44 | yes | 7/0 | 3.40 | 105 |
| p02-spike-t0 | 2/2 | g6 / 1400 ep | 8.13s / 8.40s | 7/4/4 | 3.44→2.20 | no | 0/0/0 | 3.41 | 118 |
| p03-step-t0 | 2/2 | g0 / 200 ep | 7.02s / 13.33s | 8 | 3.44→3.44 | yes | 8 | 3.41 | 96 |
| p04-crumble-t0 | 2/2 | g0 / 200 ep | 6.87s / 10.58s | 5 | 3.44→3.44 | yes | 6 | 3.45 | 87 |
| p05-mover-t0 | 2/2 | g0 / 200 ep | 5.97s / 6.37s | 6/2/0 | 3.44→3.44 | no | 7/0/0 | 3.45 | 128 |
| p06-mixed-t0 | 2/2 | g0 / 200 ep | 7.13s / 10.92s | 8/1/0 | 3.44→3.44 | yes | 10/3/2 | 3.45 | 153 |
| p07-gap-t1 | 2/2 | g1 / 400 ep | 8.37s / 9.82s | 8/5/0 | 3.44→3.25 | yes | 2/1/0 | 3.40 | 163 |
| p08-spike-t1 | 2/2 | g3 / 800 ep | 18.32s / 24.35s | 13/4 | 3.44→3.14 | no | 0/0 | 3.41 | 104 |
| p09-step-t1 | 2/2 | g0 / 200 ep | 7.22s / 9.88s | 8/5 | 3.44→3.44 | yes | 8/6 | 3.42 | 93 |
| p10-crumble-t1 | 2/2 | g0 / 200 ep | 9.78s / 9.90s | 5 | 3.44→3.36 | yes | 2 | 3.45 | 123 |
| p11-mover-t1 | 2/2 | g4 / 1000 ep | 7.23s / 9.28s | 14/6/6 | 3.43→2.93 | no | 1/0/0 | 3.45 | 175 |
| p12-mixed-t1 | 2/2 | g0 / 200 ep | 8.43s / 13.12s | 10/2/0 | 3.44→3.44 | yes | 14/4/2 | 3.45 | 207 |
| p13-gap-t2 | 2/2 | g0 / 200 ep | 8.12s / 9.67s | 8/2 | 3.44→3.44 | yes | 8/2 | 3.41 | 149 |
| p14-spike-t2 | 2/2 | g20 / 4200 ep | 8.48s / 9.95s | 18/8/1 | 3.44→1.57 | no | 0/0/0 | 3.41 | 150 |
| p15-step-t2 | 2/2 | g0 / 200 ep | 8.55s / 13.27s | 8/6/4 | 3.44→3.44 | yes | 10/8/4 | 3.41 | 217 |
| p16-crumble-t2 | 2/2 | g3 / 800 ep | 8.45s / 10.23s | 8/6/0 | 3.44→3.07 | no | 4/1/0 | 3.45 | 136 |
| p17-mover-t2 | 2/2 | g64 / 13000 ep | 7.98s / 10.67s | 49/34/11 | 3.44→0.72 | no | 1/1/0 | 3.45 | 461 |
| p18-mixed-t2 | 2/2 | g7 / 1600 ep | 11.62s / 18.57s | 20/12/5 | 3.44→3.25 | no | 4/2/0 | 3.45 | 326 |
| p19-gap-t3 | 2/2 | g1 / 400 ep | 10.02s / 10.70s | 12/8/6 | 3.43→3.34 | no | 8/0/0 | 3.41 | 388 |
| p20-spike-t3 | 0/2 | — | — / — | 44/23/14 | 3.44→1.84 | no | 0/0/0 | 3.40 | 328 |
| p21-step-t3 | 2/2 | g0 / 200 ep | 9.43s / 13.30s | 8/6 | 3.44→3.44 | yes | 7/4 | 3.42 | 161 |
| p22-crumble-t3 | 2/2 | g2 / 600 ep | 10.70s / 11.35s | 8/6/2 | 3.44→3.20 | no | 2/2/1 | 3.45 | 331 |
| p23-mover-t3 | 0/2 | — | — / — | 60/42/6 | 3.44→1.43 | no | 2/1/0 | 3.45 | 1258 |
| p24-mixed-t3 | 2/2 | g1 / 400 ep | 9.20s / 11.45s | 10/6/0 | 3.44→2.93 | no | 3/0/0 | 3.45 | 314 |

Totals: **pack 21/24 seed-runs solved, battery 12/14, generated 44/48**; the random arm won at
generation 0 on 1 pack level, 4 battery levels and 11 generated levels. The gate is a recording
gate and it passes: every metric it names exists for all 43 levels and both arms. All 24 generated
levels are certified beatable and routable (oracle 500 ms total, planner 5.92 s total in release).

### What the table says

* **The suite spans reflex to wall.** Eleven of the twenty-four generated levels are won by a
  random population at generation 0 — tier 0 and 1 are mostly reflexes. At the other end,
  `p20-spike-t3` and `p23-mover-t3` are unsolved at 100 generations on both seeds, and
  `p17-mover-t2` needs 64 generations (13,000 episodes). That is what a practice suite is for:
  a floor to check the telemetry's easy cases, and walls for the later phases to move.
* **The per-leg census is the reading fitness cannot give.** `l11` has zero solves but 68/35/18 %
  of the population crossing the first three legs: the run is not flailing at the spawn, it is
  piling up deeper. `p23` is 60/42/6 % with zero solves: the wall is the last two legs, not the
  first. `bat-pit-bait` is 17/6/6 %. Those are the sentences a level designer can act on, and
  they are invisible in a max/avg/min row.
* **Entropy is diversity, not progress.** Every generation-0 row is 3.40-3.45 bits of 3.46, so the
  probe is doing its job as a "how spread out is the population" reading. The hard runs collapse
  (`l11` 3.44→0.14, `p17` 3.44→0.72, `p23` 3.44→1.43) while the easy runs that stop at generation
  0 stay at 3.44. A collapsed population is therefore not evidence of learning, and an unchanged
  one is not evidence of failure — the later phases must gate on per-leg reach and solves, not on
  entropy alone.
* **The clock's two numbers are different questions.** `l5` has a 7.48 s best against a 25.78 s
  median, and `l2` 12.77 s against 20.95 s: a few fast winners in a slow population, which is the
  mastery league's cue rather than a level solved uniformly. On `p17` the best clock is 7.98 s but
  the median 10.67 s at the generation the run finally solved — quick for the winner, not yet for
  the population.
* **The known walls are unchanged under this protocol.** `l11` and `bat-pit-bait` are 0/2, `l9` is
  1/2, `p20`/`p23` are 0/2. One episode per genome and 100 generations is not the difficulty
  ladder's protocol; it is the comparable baseline Phase 1 inherits. The suite's hardest rows are
  the ones Phase 2's "≤ 1/3 the generations" gate should be measured against (e.g. `p17`: 64
  generations today, target ≤ 21).

### Limits

Two seeds and a 100-generation cap: this is a recorded baseline, not a significance test, and no
arm is being claimed as an improvement. The suite is deterministic by design (no patrols, flyers
or turrets) so its per-leg curves are not phase noise; stochastic mechanics stay the battery's job.
`plan ms` is the timed planner's search cost before Phase 1's compiler existed (the suite certifies
at depth 48, the pack and battery at the shipped default); the compiler and its margins are the
next section. The two front-ends' existing numbers (max/avg/min, the archive, the chart) are
untouched; telemetry is an addition, not a replacement.

Files: `crates/evo/src/telemetry.rs`, `crates/sim/src/practice.rs`, `levels/practice/` (24 files),
`tools/phase0_baseline.py`; wiring in `crates/evo/src/ga.rs`, `crates/train-cli/src/{commands,watch,main}.rs`,
`crates/app/src/{lab,panel}.rs`.

## Phase 1: the route plan, and the warm start it replaces

The route compiler now exists. `sim::route_plan::compile` certifies a level **twice** — the
reachability oracle says the flag can be reached, the timed planner returns a block-aligned route,
and that route is replayed through the real `Sim::step` and must win — then cuts the route into the
legs the telemetry already counts. `train-cli routeplan` prints the plan and writes it as JSON; the
window's generation-0 warm start now clones it instead of walking the tick-indexed route.

### The artifact

* **A leg is a span, not a crossing.** Leg `k` is the blocks from crossing `k` (or the spawn) to
  crossing `k+1` — and on a route that reaches the flag before closing the level's last rung, the
  last leg is that finish span, with no crossing of its own. Its index is therefore exactly the
  telemetry's `best_end_leg`: a body that has closed `k` crossings is in leg `k`.
  `route_plan::leg_count` is one function, and `Telemetry::measure` reads it, so the census and the
  compiler cannot disagree about what a leg is. Checkpoints are not legs; a checkpoint is a reward
  zone and a leg is a geodesic crossing.
* **Two boundary cases are pinned rather than smoothed over.** A block that lands two rungs closer
  closes two crossings: the plan reports two legs, the second with zero blocks and zero frames. A
  route that ends on the flag without closing every rung gets a finish leg with `hops == 0`.
  Merging either would keep the legs contiguous but make a plan's index off by one against the
  telemetry, which is the whole property the artifact exists for.
* **A margin is time, and it is measured.** Each leg's `margin_frames` is
  `budget_frames - end_frame` at the leg's end: the frames still in the episode's budget when the
  crossing completes. That is the slack a learner has before the clock runs out, and it is the one
  margin the planner can actually price — the search works in blocks and frames, not in clearance
  to a hazard, and a geometric margin the planner never measured would be a number without a
  witness. Margins are non-increasing along a plan, and the last leg's is the plan's own win
  margin.
* **Three failures, kept apart.** `Unbeatable` (the oracle says no), `Unroutable` (the oracle
  certifies the level, this planner budget found no line; `beatable()` is true) and
  `ReplayFailed` (the planner's route did not survive the sim's own replay). A warm start reads
  the distinction: it reports the failure in the run's origin note and starts **random**. It never
  falls back to the tick-indexed route walk, which is the weaker demonstration this replaced.

### The warm start

The window's `RunPlan::warm_from_plan` (formerly `warm_from_route`) compiles the plan at depth 64 —
`l11-the-elements` needs 64 for its 11-rung route, and the planner's shipped default of 32 is one
expansion short — and clones it with `from_plan`'s block-aligned fit plus its six DAgger rounds at
the window's 300-epoch budget. `evo::from_certified_plan` is the split that lets the compiler be
paid for once: the window decides *whether* there is a plan, then clones the certified route.
`train-cli --from-route` still exists as the older arm; `--from-plan` is the thorough terminal
version (10× the epochs).

Measured cost, release build, one level: the compiler is 0.07–0.42 s per practice level (all 24 in
**5.25 s**); the full plan + DAgger warm start is **6.16 s** on `l11` and **9.4 s** on
`l2-first-gap` at 300 epochs. It runs once, but it runs synchronously at run start, so the window
is unresponsive for that long on a hard level.

### The gate, as run

`tools/phase1_gate.py --gens 60 --seeds 1`, pop 200, one episode per genome, stop on first solve,
curriculum off.

**A. The artifact.** `routeplan --all` over the practice suite plus `l11`, at depth 64; the script
checks the written JSON rather than the printer: one leg per span (`legs == hops`), legs contiguous
from block 0 to `actions.len()`, `sum(frames) == frames`, `sum(hops) == covered`, margins
non-increasing with the last equal to the win margin.

| level | legs | rungs closed | blocks | frames | slack at leg 1 | win margin |
|---|---|---|---|---|---|---|
| l11-the-elements | 12 | 11/12 | 183 | 731 | 2808 | 2149 |
| p01-gap-t0 | 2 | 2/2 | 99 | 395 | 2488 | 2165 |
| p06-mixed-t0 | 3 | 3/3 | 100 | 397 | 2292 | 2003 |
| p14-spike-t2 | 3 | 3/3 | 127 | 508 | 3088 | 2652 |
| p17-mover-t2 | 8 | 8/8 | 118 | 469 | 2948 | 2571 |
| p19-gap-t3 | 5 | 5/5 | 145 | 578 | 3408 | 2942 |
| p20-spike-t3 | 6 | 6/6 | 153 | 609 | 3316 | 2791 |
| p23-mover-t3 | 11 | 11/11 | 230 | 919 | 3368 | 2521 |
| p24-mixed-t3 | 4 | 4/4 | 145 | 580 | 3292 | 2900 |

(The other sixteen rows are in the script's output; every row passes the same assertions.) **25 of
25 plans compiled** — 24/24 practice levels and `l11` — with exact boundaries and margins.
`l11` is the one route that reaches the flag with a rung unclosed (11 of 12): its last leg is the
finish span. `l9-wait-and-climb` is the oracle-certified, planner-unroutable control: refused at
depth 64 and still refused at 128 ("beatable in principle, but no timed route after 128
expansions"), which is the report the warm start gives instead of cloning the walk.

**B. The warm start.** Three arms on the suite's hard tail plus `l9` and `l11`; `g0/first` is the
solved count at generation 0 and the first generation that solved anything.

| level | legs | plan clone | random g0 / first | route g0 / first | plan g0 / first |
|---|---|---|---|---|---|
| p02-spike-t0 | 3 | died | 0/200 / g9 | 0/200 / g5 | 1/200 / g0 |
| p14-spike-t2 | 3 | stuck | 0/200 / g21 | 0/200 / g7 | 0/200 / g1 |
| p17-mover-t2 | 8 | died | 0/200 / — | 0/200 / g16 | 4/200 / g0 |
| p18-mixed-t2 | 5 | died | 0/200 / g3 | 0/200 / g7 | 0/200 / g1 |
| p19-gap-t3 | 5 | won | 0/200 / g1 | 26/200 / g0 | 26/200 / g0 |
| p20-spike-t3 | 6 | died | 0/200 / — | 0/200 / g37 | 0/200 / g2 |
| p22-crumble-t3 | 5 | stuck | 0/200 / g2 | 0/200 / g1 | 5/200 / g0 |
| p23-mover-t3 | 11 | died | 0/200 / — | 0/200 / — | 0/200 / — |
| l9-wait-and-climb | — | — | 0/200 / — | 0/200 / — | no plan (reported) |
| l11-the-elements | 12 | died | 0/200 / — | 0/200 / — | 0/200 / — |

Over the nine levels with a plan, **generation 0 solved: random 0, route 1, plan 4** (p02, p17,
p19, p22). Where the plan arm did not solve outright it still reached a first solve sooner than
both alternatives — p14 g1 against g21/g7, p18 g1 against g3/g7, p20 g2 against never/g37 — with
two exceptions: p19 ties at g0, and p23 is unsolved by all three arms at 60 generations.

### What the numbers say, and what they do not

* **The plan is the warm start the route walk was not.** Four of nine levels are solved by
  generation 0 from the plan clone, against one from the walk and none from random; on the spike
  and mixed tiers the plan arm's first win is 3–10× sooner. That is the Phase 1 claim, measured on
  the suite Phase 0 certified.
* **`l11` is honest about its limit.** The plan routes (12 legs, 11 rungs, 35.8 s of its 48 s
  budget unspent) and the window now uses it, but at 300 epochs the clone reproduces 84.7 % of the
  decisions and dies on the first spike; no arm solved `l11` in 60 generations. The CLI's 3 000
  epochs is the budget where the plan's clone reaches the second deck, so closing this needs
  either that budget (≈60 s of startup) or Phase 2's leg learner — not a weaker fallback.
* **The reported refusal works.** `l9` starts random and says why; it is not silently warmed from
  the walk (which also solves nothing on it in 60 generations at pop 200).
* **Cost is real.** The compiler is cheap; the DAgger warm start is not, and it blocks run start
  for 6–9 s in the window. Moving it off the UI thread is the natural next step, not done here.

### Gates

`cargo clippy --workspace --all-targets -- -D warnings` is clean. `cargo test --workspace
--no-fail-fast` runs all 21 test binaries and passes every test except one: `evo --test migration`'s
saved-`l11` replay. That failure is the **working tree's** uncommitted edit to
`levels/l11-the-elements.json` (an added obstacle chain), not this pass: the fixture's champion was
recorded on HEAD's terrain (1567 frames) and the edited level ends the same policy at 49. Temporarily
restoring HEAD's file makes the binary pass 3/3; the file is left exactly as it was found. Nothing
in Phase 1 depends on that level's terrain — `l11`'s plan was compiled against the edited file and is
the one quoted above.

Files: `crates/sim/src/route_plan.rs` (artifact and compiler), `crates/evo/src/imitate.rs`
(`from_certified_plan`), `crates/evo/src/telemetry.rs` (shared `leg_count`, per-level record),
`crates/evo/src/ga.rs` (per-level suite pass and `GenerationReport::per_level`),
`crates/app/src/lab.rs` (plan warm start, refusal note), `crates/train-cli/src/commands.rs`
(`routeplan`, per-level JSONL), `tools/phase1_gate.py`.

## Phase 2: leg skill, taught where the clone fails

Phase 1's teacher was a route and a fit. Measured on `l11`, that clone reproduced 84.7 % of the
plan's decisions and died on the first spike inside leg 1, and the suite's agreement number could
not say *where* it stopped. Phase 2 makes the teacher answer the student and makes the reading
per leg.

### The teacher: replanning, one crossing at a time

* **A leg target, not the flag.** `sim::plan::plan_leg` is the shared planner search stopped at the
  k-th route crossing (`route_hops_closed`) instead of the flag. A correction from a divergent state
  only has to finish the leg the student is standing on; asking for the whole level from there made
  the search fail at any realistic depth (measured: every replan returned `None`, so every label
  fell back to the plan's nearest state, and the first attempt at this pass cost 128 s while
  changing nothing).
* **Corrections come from the student's state.** A rollout step asks the planner for an action
  when the observation is off the demonstration's corridor (`squared sensor distance > 0.5`), and
  replans from *that* snapshot. On the corridor the demonstrated action is the label — asking the
  planner at those states was measured to replan on every block while returning the same action.
* **Refits continue from the student.** `imitate::clone_from_play_init` initialises from the
  current clone (at a quarter of the from-scratch learning rate) instead of a fresh random net.
  Refitting from scratch at a few hundred passes was measured to lose a leg every round; the
  round was rejected and nothing improved.
* **Dataset segments reset recurrent state.** Every lesson rolled out from a snapshot is a new
  episode, so `RoutePlay::segments` marks its start and the fit zeroes the feedback context there.
  Without it the first lesson was trained with the memory left over from the flag.
* **Two whole-route DAgger rounds, then the curriculum.** Six from-scratch rounds were the Phase 1
  shape; with corrections planned locally, two rounds reach the same wall, and the budget is
  better spent on the failing leg.

### The curriculum and the reading

`measure_mastery` rolls the clone out from the spawn on the plan's episode and reports, per leg,
whether it was reached, whether its crossing was cleared, whether the episode ended in it, and the
frames spent there. `weakest()` is the first leg without a crossing — the telemetry's `best_end_leg`
and the plan's `Leg::index` are the same number, so the reading needs no translation. The curriculum
then rolls out from the leg **before** the weak one (the approach contains the failure; starting on
the weak leg gave four states on `l11` and taught only the death), from the state the student
actually reaches, labels the divergent states with `plan_leg`, refits from the student, and adopts
the round only if it clears more legs. A round that teaches nothing is discarded, not returned.

### The gate, as run

`tools/phase2_gate.py --seeds 8`: `l11`, plan depth 64, clone 300 epochs (the window's budget), pop
8; three arms differing only in the teacher. Distributions, not one number:

| arm | mean / median / min-max legs cleared | past the first spike | deaths by spike | plan+fit s |
|---|---|---|---|---|
| imitate (Phase 1) | 1.12 / 1.0 / 1-2 | 1/8 | 7/8 | 2.88 |
| replanning DAgger | 1.88 / 2.0 / 1-3 | 6/8 | 2/8 | 3.88 |
| + leg curriculum | **2.00** / 2.0 / 1-4 | **6/8** | 3/8 | 5.70 |

Per seed, the leg the mastery reading named:

| seed | imitate | DAgger | curriculum |
|---|---|---|---|
| 1 | 1, died spike x 16.3 | 2, died turret x 16.2 | 2, died turret x 16.2 |
| 2 | 1, died spike x 16.3 | 2, died turret x 16.2 | 2, died turret x 16.2 |
| 3 | 2, died turret x 16.3 | 3, stuck x 25.4 | **4, died spike x 28.1** |
| 4 | 1, died spike | 2, died turret | 2, died turret |
| 5 | 1, died spike x 16.6 | 2, died turret | 2, died turret |
| 6 | 1, died spike x 16.3 | 1, died spike | 1, died spike |
| 7 | 1, died spike x 16.3 | 1, died spike | 1, died spike |
| 8 | 1, died spike x 16.4 | 2, died turret x 16.3 | 2, died turret x 16.3 |

**The named failure moved.** The Phase 1 clone died on the first spike in 7 of 8 seeds; with the
replanning teacher 6 of 8 clear that crossing and now stop at the turret one leg later, and seed 3
clears four legs. The remaining two seeds (6, 7) still die on the first spike, and the mastery
reading names leg 1 for both — which is the required answer when the teacher does not get there.
The curriculum's own contribution is small and honest: +0.12 legs on average, one seed (3) gaining
a leg over DAgger, every other seed unchanged, and it costs about 2 s of startup.

### What this does not claim

* **The turret is the new wall, and the curriculum does not clear it.** All improved seeds stop at
  x 16.2 in leg 2 with the same death. A leg lesson there contains the approach and the shot but
  the 300-epoch fit does not learn the wait; the round that tried was measured and rejected. Phase
  2's claim is the spike, not the level.
* **Two seeds show no gain at all.** The gate reports them rather than averaging them away.
* **Cost is real.** Replanning adds ~0.9 s and the curriculum ~2 s to startup; both are inside the
  synchronous warm start, which the window still runs on its worker before generation 0.
* **`l11`'s plan is compiled against the working tree's level file**, which is the user's
  uncommitted edit; the gate's numbers are about that terrain.

### Gates

`cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` green except
the pre-existing `evo --test migration` saved-`l11` fixture, which fails only because that level file
is edited in this working tree (it passes on HEAD's file, as recorded in Phase 1).

Files: `crates/sim/src/plan.rs` (`plan_leg`, leg-targeted search), `crates/evo/src/imitate.rs`
(replanning DAgger, `segments`, `clone_from_play_init`, `LegMastery`, curriculum),
`crates/evo/src/route_play.rs` (segment starts), `crates/train-cli/src/{commands,main}.rs`
(`--dagger-rounds`, `--leg-rounds`, `--leg-rollouts`, `--replan-depth`, mastery on the `clone` line),
`crates/app/src/lab.rs` (mastery in the run's origin note), `tools/phase2_gate.py`.
