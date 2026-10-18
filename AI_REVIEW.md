# AI review — October 2026

**Scope:** `crates/evo` (genome, GA, ES, fitness, curriculum, imitation, archive) and the
learning-facing half of `crates/sim` (sensors, episode stats, the stuck rule), plus how the two
front-ends drive a run.

**Baseline machine** (the "consumer hardware" bar for every number below): Intel Core i5-9400,
6 cores / 6 threads, 2.9 GHz base / 4.1 GHz turbo, 15 GiB RAM, AVX2 + FMA, no AVX-512.

**Method.** Read the learning code and the three design documents; built `--release`; ran the whole
suite; then took four measurements of my own because the documents do not answer them — generation
throughput and its shape sensitivity, the forward pass's cost per decision, the reliability of the
champions a run actually saves, and whether population breadth or generation depth buys a win more
cheaply. Commands and raw numbers are in the appendix. `cargo test --workspace` is green on this
tree: **680 tests, 679 passed, 0 failed, 1 ignored**.

This document does not repeat the history in [REVIEW.md](REVIEW.md) §4 — that section already
records what each mechanism was measured at. It asks the one question the documents have not:
*given that the search is not the binding constraint, where is the learning loop actually leaving
intelligence on the table?*

---

## 0. Verdict

The learning loop is unusually well engineered for a project of this size: a deterministic
fixed-topology GA, a versioned trio that refuses stale champions rather than lying about them, a
fitness function whose every constant is derived from the physics, a measured stall→detour chain,
and a hand-rolled ES baseline that proves the GA is doing real work. The remaining headroom is
**not** in the search algorithm. It is in five places, in order of what they cost you:

1. **The loop does not ask for reliability.** "Solved" means one genome won *one* patrol phase, and
   the champion a run saves wins **50–67 % of unseen episodes** (measured, §2.1). The trials
   machinery exists and is wired into *ranking* but never into `Outcome::Won`, `wins`,
   `--stop-on-win` or the saved artifact.
2. **What the run keeps is chosen by a maximum over re-rolled worlds.** `best_ever`, the archive's
   incumbents and the stagnation counter all compare scores taken on *different* patrol episodes, so
   they are optimistically biased by construction (§2.2).
3. **Two of the three mutation scales are mis-scaled.** "Fine" children are *sparse*, not small: 1–3
   genes moved by `N(0, 0.30)` on weights whose own scale is 0.13. They still pay (§2.3), which means
   the second scale is real — it is just being used as a re-roll where the literature and the code's
   own comments want a nudge. The archive's children are bred with the *coarse* move that destroys
   exactly the niche the archive exists to preserve (§2.4).
4. **The hot path is the network, and it is running at scalar speed.** A decision costs 1.79 µs —
   3.7 cycles per MAC, i.e. a serial FP dependency chain — and the forward pass is roughly **half of
   a generation**. Four accumulators alone halve it (measured); lane-per-genome SIMD should give 4–6×
   with **bit-identical** results (§2.5).
5. **None of the sophistication is reachable from the game, and the trainer is idle.** The window
   builds `GaConfig { ..GaConfig::default() }` — no memory, no width, no trial count, no novelty —
   and at 15–19 ms per generation it sits idle for 95 %+ of a watched run (§2.6). On this hardware the
   AI can be materially better *inside the same frame budget*.

Below the top five: the observation cannot express time-to-event for any of the five new mechanics
(§2.7); patience is still capped below a patrol round trip (§2.8); the demonstration path is blocked
by a cadence mismatch that wants a small sweep rather than a search (§2.9); and several shipped
"measured negatives" are under-powered relative to the batch variance this repo itself documented
(§2.10).

---

## 1. The loop as it stands

| Layer | Mechanism | Default | Where |
|---|---|---|---|
| Policy | fixed-topology MLP, 100 → 16 → 11, `tanh`, argmax | 1 803 genes | [genome.rs](crates/evo/src/genome.rs) |
| Memory | first hidden layer fed back into the next observation | **off** (`feedback 0`) | `NetShape::feedback` |
| Observation | 9×9 patch + 19 scalars (goal, enemy triple, gaps, wall, sprint, waypoint, sub-tile phase) | sensors v12 | [sensors.rs](crates/sim/src/sensors.rs) |
| Control | macro-action held for 4 ticks | `DECISION_TICKS 4` | [lib.rs](crates/sim/src/lib.rs) |
| Search | tournament GA, uniform crossover, Gaussian mutation, 4 elites | sigma 0.30, rate 0.05 | [ga.rs](crates/evo/src/ga.rs) |
| Second/third scale | 8 fine children of the best; 8 reheated immigrants after 20 flat generations | on | `Trainer::breed` |
| Quality-diversity | MAP-Elites, 8 progress × 6 checkpoint × 5 time = 240 cells | fills, but is not bred from | `MapElitesArchive` |
| Selection extras | pessimism (mean→floor), rarity bonus | both 0 | `selection_scores` |
| Episode policy | one world per generation, successive-halving trials for survivors | on, 1.75 episodes/genome | `evaluate_suite_raced` |
| Fitness | progress + 32/hop + 16/checkpoint − 2/jump − 20/death + 10 000/win + 200·speed | v8 | [fitness.rs](crates/evo/src/fitness.rs) |
| Escape | 40-generation detour on the derived practice level, seeded with jump-timing takes | on | [curriculum.rs](crates/evo/src/curriculum.rs) |
| Demonstration | oracle walk → behaviour clone → a quarter of generation 0 | `--from-route` | [imitate.rs](crates/evo/src/imitate.rs) |

That is a good machine. Everything below is about what it is pointed at and what it keeps.

---

## 2. Findings, ranked

### 2.1 "Solved" is a single draw, and the saved champion is a coin flip — **the headline**

**What the code does.** `GaConfig::effective_trials` judges a walker level on three patrol phases,
and `evaluate_suite_raced` really does run them. But:

* `EpisodeResult::won()` reads `outcome`, and `outcome` comes from the **representative** episode —
  trial 0 of the first level ([fitness.rs:445](crates/evo/src/fitness.rs#L445),
  [ga.rs:981](crates/evo/src/ga.rs#L981)).
* `advance` counts `wins` with that predicate ([ga.rs:1730](crates/evo/src/ga.rs#L1730)), and
  `train --stop-on-win` stops on `report.wins > 0`
  ([commands.rs:317](crates/train-cli/src/commands.rs#L317)).
* The *only* conservative "won every trial" signal is `suite_wins` — and it is gated on
  `levels > 1`, so **a single walker level has no conservative signal anywhere**
  ([ga.rs:988](crates/evo/src/ga.rs#L988)).

So on the one level class where selection is genuinely noisy, a run calls itself solved when one
genome happened to draw a good phase, and the champion it writes is the top-*mean* genome, which need
not be that genome.

**Measured.** Six `--stop-on-win` runs at pop 200, then each saved champion re-scored on 12 unseen
patrol phases (4 master seeds × 3 trials):

| level | run seed | first-win gen | champion wins, unseen phases |
|---|---|---|---|
| `l9-wait-and-climb` | 1 | 81 | 6 / 12 |
| `l9-wait-and-climb` | 5 | 46 | **12 / 12** |
| `l9-wait-and-climb` | 9 | 227 | 6 / 12 |
| `l3-gauntlet` | 1 | 23 | 6 / 12 |
| `l3-gauntlet` | 5 | 28 | 6 / 12 |
| `l3-gauntlet` | 9 | 13 | 6 / 12 |

**42 / 72 = 58 %.** And the failure is not marginal: the l9 champion from generation 38 produced
exactly **two trajectories** across 24 fresh episodes — `best_x 27.3, 260 frames` (dies at the patrol)
or `best_x 56.5, 732 frames` (wins). The policy has learned one crossing strategy that works when the
patrol happens to be where it needs it, and nothing else. That is not a lottery in the sense the
documents worry about; it is a *phase-dependent* policy being scored on three draws.

**Why it matters.** The champion is the artifact the game races, the ghost, the splash, and the thing
"the AI beat this level" means to a player. At 58 % it is a policy that beats the level about half
the time, presented as a solve. Every downstream claim inherits this — including the level pack's
difficulty numbers, which are "generations until *some* genome won *one* phase".

**Fix.** One concept, applied in one place:

1. Add `EpisodeResult::solved` (or `won_all_trials`) computed from the tally's `won` vector for **all**
   levels, not just suites, and use it for `wins`, `--stop-on-win`, and the panel's "reached the flag".
2. Make the champion a **validated** artifact: when a candidate beats `best_ever`, re-run it on a
   fixed held-out seed set (say 8 episodes, seeds derived from `master_seed` and *not* from the
   generation) and store that mean. Keep the per-episode number for `replay` as it is.
3. Expose the reliability number: "beats this level on 11 of 12 trials" is a far better thing for the
   panel to say than "won".

**Cost.** No physics, no sensors, no fitness change → **no version bump, no champion invalidation**
(this is all in the trainer's bookkeeping). The held-out set costs 8 episodes per candidate *only
when a record is broken*, which is a rounding error beside a generation.

**The measurement that would settle it.** Re-run the difficulty table with `solved` as the metric.
Expect first-win generations to rise on walker levels (the runs will need more phases to agree) and
the champion column to jump from ~58 % to >90 %. If it does not, the level is a coin flip for a
different reason and that is worth knowing too.

---

### 2.2 `best_ever`, the archive and the stall counter compare different worlds

**What the code does.** `episode_seed_for` mixes the generation into the seed
([ga.rs:856](crates/evo/src/ga.rs#L856)), so even under `common_episodes` the *world re-rolls every
generation*. Three records then take maxima across generations:

* `best_ever` keeps the genome with the highest `mean_fitness` ever seen
  ([ga.rs:1823](crates/evo/src/ga.rs#L1823));
* `MapElitesArchive::observe` replaces an incumbent when `r.mean_fitness > incumbent.fitness`
  ([ga.rs:1309](crates/evo/src/ga.rs#L1309));
* `stagnation` resets on `max_fitness > last_max`, so a lucky phase reads as progress and postpones
  the detour.

A maximum over noisy draws is upward-biased: with three episodes of a ~50 %-reliable policy, the max
of the *generation* is a good draw, not a good policy. Pairing fixed the noise *inside* a generation
and REVIEW.md says so explicitly — but every record above still lives across generations.

**Fix.** Same mechanism as §2.1: a fixed **validation episode set** (`master_seed` + a constant, no
generation), used for `best_ever`, for the archive's replacement test, and for the stagnation
comparison. Cost: `k` extra episodes per generation for the archive (~240 cells, but only the
challenger needs re-running) — or restrict it to the top-8 genomes, which is cheap and enough.

**Cost.** Trainer bookkeeping only. No version bump.

---

### 2.3 The "fine" scale is sparse, not small

**What the code does.** Fine children perturb `1..=3` genes of the generation's best by
`N(0, sigma)` with the **same** `sigma` as a coarse child ([ga.rs:2083](crates/evo/src/ga.rs#L2083)).
Xavier initialisation puts a gene's own scale at `sqrt(6/(100+16))/sqrt(3) ≈ 0.13`, so a fine step is
≈ **2.3 standard deviations of the gene**: the fine child does not nudge the boundary, it re-draws it.
The module's own docs are explicit that the intent is "a step, not a lottery ticket" (see
`mutate_few`).

**Measured** (l9, pop 200, 200-generation budget, first-win generation, 8 seeds):

| arm | per-seed first win | median |
|---|---|---|
| shipped (fine children on) | 81 28 38 104 46 38 51 63 | **44.5** |
| `--fine-children 0` | 89 41 82 78 44 51 51 53 | 51.0 |

So the second scale **is** earning its place — removing it is worse on 5 seeds, better on 2, tied on 1.
The conclusion is not "remove it", it is **"it is running at the wrong temperature"**. The fix is to
give it its own step, e.g. `fine_sigma = 0.25 × mutation_sigma` (0.075 at the shipped 0.30), which is
inside the noise of a single gene instead of outside it.

**Cost.** Search only. Add `fine_sigma_scale` to `GaConfig` (serde-defaulted so old champion files
read back); no version bump, no retrain.

**Measurement.** The same 8-seed protocol at 0.1 / 0.25 / 0.5 × sigma, then 24 seeds on l9 and
`scratch-spike-tight` before shipping a default. The 44.5-vs-51 result above is one batch of eight and
the repo has measured a 5-seed spread between batches of twelve — treat it as a direction, not a size.

---

### 2.4 Archive children are bred with the move that destroys the niche

**What the code does.** The archive exists to keep one genome per behavioural niche alive, so a
stepping stone that plain ranking would delete can still be a parent. Its children are then bred with
`mutate(&mut child, sigma, rate, ...)` ([ga.rs:2163](crates/evo/src/ga.rs#L2163)) — the **coarse**
move, ~86 genes at the default rate, exactly the mutation a niche incumbent is least able to survive.
`--archive-children` was then measured as a loss on the plateau (10/12 → 7/12) and turned off by
default, with `--archive-on-diversity` recovering most of it.

That reading is consistent with the mechanism being *mislabelled*: an archive that keeps a genome and
then re-rolls five percent of it is not preserving a niche, it is paying a slot for a random child of
a below-average parent. The fine step (`mutate_few`, 1–3 genes) is the move that would preserve the
behaviour and improve it; it is already in the file and is used two hundred lines earlier.

**Fix.** Archive children get 1–3 fine genes, like fine children, optionally at the fine sigma from
§2.3. Then re-measure `--archive-children 2` (ungated) on the plateau. The measured loss may well
disappear; if it does, the archive becomes the search mechanism it was designed to be rather than a
reported one.

**Cost.** Search only, no version bump. This is the cheapest untested mechanism in the tree.

---

### 2.5 The network is the hot path, and it is running at scalar dependency-chain speed

**Measured.** On the baseline machine, one generation of 200 genomes on `l9` (1.75 episodes each):

| arm | ms per generation | reading |
|---|---|---|
| shipped `100>16>11` | **19** | 10 363 population-episodes/s |
| `--hidden 32` | 27 | |
| `--hidden 64` | 46 | 2.4× the shipped width |
| `--hidden 32,16` (two layers) | 27 | |
| `--feedback 8` | 19 | **free** |
| `--no-auto-trials` (1 episode) | 10 | half the episodes, half the time |
| `--action-repeat 2` | 25 | twice the decisions |

Fitting `time = floor + net` to the first and third rows: **the forward pass is ≈ 9 ms of the 19 ms
generation**, and it is ~1.79 µs per decision. Isolating the inner loop on a 1 600-MAC row:

| inner loop | ns per row | cycles/MAC |
|---|---|---|
| one accumulator (as shipped, `dense`) | 1 687 | **3.7** |
| four accumulators | 850 | 1.9 |
| eight accumulators | 801 | 1.8 |

3.7 cycles per multiply-add is a **serial dependency chain** — each `acc += w*x` waits on the last —
not a throughput limit. The 2× from four accumulators is free performance the compiler cannot find
because reassociating a float sum is not allowed without a fast-math flag.

**The important caveat, and the shape of the right fix.** Reassociation *changes the last bits*
(1 672 315.12 → 1 672 314.88 in the benchmark), which can flip an argmax on a boundary and therefore
invalidates champions. So do **not** unroll `dense`. Instead vectorise **across genomes**: all
genomes share a shape, so eight genomes can be evaluated with one f32x8 lane per genome, each lane
accumulating in exactly the shipped order. Results are bit-identical per genome, determinism is
untouched, and the win is the full width of the ISA (AVX2 here, NEON on Apple silicon, scalar
fallback elsewhere):

* repack the generation into a lane-major weight buffer once per generation (~200 × 1 803 floats,
  a few hundred microseconds);
* `for each layer, for each output row: 8 lanes of FMA over fan_in`;
* keep the existing scalar path as the reference implementation and assert the two agree on a
  fixture — that assertion is the whole safety story.

**What it buys.** Roughly 1.7× generation throughput at the shipped shape, or — more interesting —
the same 19 ms with a 4× wider or recurrent net, at the width where §2.7's sensor additions live.

**Second-order wins, all cheap:** `EpisodeRunner::new` takes the `Genome` **by value**, so every
episode in a generation clones it (~7 KB × 350 per generation,
[fitness.rs:475](crates/evo/src/fitness.rs#L475)) — an `Arc<Genome>` or
a borrowed runner removes it; `step_verbose` + `Packet` clone the whole population and every
`EpisodeResult` once per displayed generation (~1.2 MB) — the window only needs the lanes and the
replay's ghosts, not all 200.

---

### 2.6 The game's trainer is the *least* capable configuration, and it is idle

**What the code does.** The window starts every run from defaults
(`GaConfig { ..GaConfig::default() }`, [app.rs:783](crates/app/src/app.rs#L783)): memoryless, 16-wide,
one trial rule, no novelty, no archive children. The keys expose pause, step, speed, mutation rate,
scrub, save. Every capability the last four passes added — `--feedback`, `--hidden`, `--rank-pessimism`,
`--novelty`, `--archive-children`, `--suite`, `--from-route` — is CLI-only.

**Measured.** At 8× speed a generation's animation is 0.6–5 s; generation cost is 0.015–0.026 s. The
worker is also *deliberately* capped at one generation of lookahead
([lab.rs:1560](crates/app/src/lab.rs#L1560)), so it is idle **95–99 %** of a watched run on this
machine.

**Fix, in two parts.**

* **A "thinking" dial.** Let the window run `k` generations per displayed generation (k = 1/2/4/8),
  or run `k` independent populations and show the best. The budget is already there; the panel can
  say what the dial costs ("4 generations per step: 78 ms of work per generation shown").
* **Capability in the window.** At minimum: memory width (0/8/16) and trials (1/3), which are the two
  settings that change *what the AI can be* rather than how it searches. Both are already in
  `GaConfig`; the panel has room in the Details board, and the run header already prints the net.

The "best of N restarts" version is the cheapest reliability win on 6 cores: with four 100-generation
islands at the same total episode budget, the measured solve pattern on l9 was 23 of 32 sub-runs
winning — i.e. **best-of-4 lands a win in about a quarter of the wall clock** of one long run, which
is what a person waiting at the window actually cares about. (Episode-efficiency does not improve:
one run of 400 generations solved 8 of 8 seeds and was ~3.8× cheaper per solve than pop 800 × 100.
Breadth buys *latency*, depth buys *episode efficiency* — the window wants latency.)

---

### 2.7 The observation cannot express time-to-event, and has no clock

The five mechanics added since 0.11 are all *timed*: a turret fires every 120 ticks, a lava vent
every 150, a flyer beats at its own speed, a mover travels at 1/16 tile per tick. What the observation
carries about them is **position only** ([sensors.rs `observe_scene`](crates/sim/src/sensors.rs)): the
nearest threat's `dx, dy` and heading, and a mover drawn into the patch as one-way ground.

A memoryless policy therefore cannot see:

* **how long until a turret fires** (its cooldown), so "wait, then run" is unexpressible;
* **which way a mover is going / how fast**, so "stand here and the shelf will arrive" is unexpressible
  (the patch says where it *is*, not where it will be);
* **the phase of a lava vent**, so the arc is a surprise every period;
* **its own elapsed time and progress** — no `frames/budget`, no "how far have I got", so patience and
  commitment have nothing to key on. (This is the same class of gap as the v7 walker heading: a fact
  about the world that no amount of training can recover.)

Each of these is one or two scalars, and the derived quantity — **time to contact** — is cheap to
compute from state the sim already has. A single "next event" triple (ticks until the nearest timed
hazard acts, its direction, the body's own clock) would make three of the five new mechanics
learnable rather than reactive.

**Cost.** Sensors version bump → champions refused, one retrain. That is the expensive kind of change,
which is why it should be bundled with §2.8 rather than done twice.

---

### 2.8 Patience (600 frames) is shorter than a patrol round trip (≈1220 on l9)

`NO_PROGRESS_FRAMES = 600` ([lib.rs:76](crates/sim/src/lib.rs#L76)) ends an episode as `Stuck` unless
the body makes a **new closest approach** or closes a **new route hop**. On `l9`, the walker patrols a
25-column floor (`walker_speed 0.04`, box 0.6 tiles): about 610 ticks each way, **≈1220 ticks round
trip**. A body whose correct answer is "let it pass and come back" can be terminated for camping
before the patrol returns.

**But the evidence says this is not currently the ceiling.** My 8-seed sweep solved l9 on 8 of 8 seeds
at 200 generations (28–104), and the earlier 6-seed sweep solved 6 of 6 at 400, so whatever the cap
costs, the population is getting around it — probably by following the walker rather than waiting.

**Fix (still worth doing, at lower priority).** Let the stuck timer be reset by the *world* advancing
as well as the body: a walker that has turned around, a mover that has completed a leg, a turret that
has fired since the last reset. That is one predicate in `Sim::step`, no observation change.

**Before spending anything on it**, run the probe the documents already ask for: for each patrol phase,
the earliest frame at which a scripted crossing is safe. If that distribution has mass beyond 600, the
cap is pricing a correct behaviour and the fix is worth a version bump; if not, the cap is fine and the
item can be closed.

---

### 2.9 The demonstration path wants a sweep, not a re-search

The docs are right that the oracle-clone fails because the oracle presses on a **tick** and the policy
decides on a **4-tick grid**, and right that forcing the replay onto decision boundaries breaks the
walk. But the proposal on the table ("a cadence-aware oracle with its own `MOVE_SET_VERSION`") reads
like a research project, and it does not have to be one.

Every canonical move in `sim::reach` has exactly **two** timing parameters that matter: the
*decision index* of the take-off and the *number of decision blocks* the jump is held. That is a 2-D
lattice of about 5 × 5 = 25 takes per crossing, each one a single scripted crossing of a few dozen
ticks — microseconds. So:

1. keep the oracle's geometric move set exactly as it is (it already proves reachability);
2. for each move, sweep `(take-off block ± 2, hold blocks 1..5)` through the real stepper;
3. keep the first take that lands, and record *that* as the demonstration.

That turns `--from-route` from a walker into, plausibly, a solver, for a bounded amount of work and no
new search. It is also the honest way to find out whether the cadence mismatch is the whole story: if
no take in the lattice lands where the tick-precise press did, the move is genuinely not expressible
on the policy's grid and the oracle should report it as such rather than the clone silently failing.

The same lattice is the ground truth a **DAgger** pass needs (run the clone, relabel the states it
actually visits with the nearest lattice take), which is the standard fix for the compounding-error
caveat in `imitate.rs`'s own docs.

---

### 2.10 Measurement hygiene: the shipped defaults rest on under-powered arms

This is a review of the review, and it matters because the next pass will build on those numbers.

* The repo measured that **the same arm, binary and level reads 6/12 on seeds 1–12 and 11/12 on seeds
  13–24** (REVIEW.md §4, the instrument section). That is a 5-seed spread between batches of twelve.
* The conclusions that changed defaults sit at or below that scale: `--archive-children 2` on the
  plateau (10 → 7 of 12), `--archive-on-diversity` (9 of 12, "gains 3, loses 5"), `--hidden 32`
  (6 vs 9 within one batch, p = 0.23), `--rank-pessimism 1.0` (6 vs 9, p = 0.75), the rarity bonus
  ("a few seeds").
* My own 8-seed arms above move by 3–6 seeds between configurations and are just as noisy.

`bench --compare` already implements the right instrument (paired, exact sign test, recorded knobs).
The gap is that the mechanisms most likely to be re-examined were not run through it at a size that
can carry a default.

**Recommendation.** Any knob that changes a *default* gets a **24-seed paired arm** through
`bench --protocol` with `--compare`, and the write-up quotes the paired discordant count rather than
the solve count. Anything smaller is a hypothesis, which is fine — it just should not be the reason a
mechanism is off.

---

## 3. Ranked roadmap

Ordered by (expected gain to the AI's competence) / (cost). "Retrain" means the version trio moves.

| # | Change | Retrain? | Expected gain | Effort |
|---|---|---|---|---|
| 1 | `solved` = won every trial; champion validated on held-out seeds (§2.1) | no | the artifact becomes what it claims (58 % measured today) and every later number gets honest | S |
| 2 | Fixed validation episodes for `best_ever`, archive, stagnation (§2.2) | no | selection stops rewarding lucky phases; less re-learning | S |
| 3 | Fine sigma separate from coarse (§2.3); archive children bred fine (§2.4) | no | cheaper polish near the jump boundary; the archive may start earning its place | S |
| 4 | Lane-per-genome SIMD forward pass + drop per-episode genome clones (§2.5) | no | 1.7× throughput, or 4× network at the same clock | M |
| 5 | Window: thinking dial + memory/trials in the UI (§2.6) | no | the in-game AI uses the hardware it is running on | M |
| 6 | Time-to-event observation slots (turret cooldown, vent phase, mover velocity, body clock) (§2.7) | **yes** (sensors) | the five new mechanics become learnable as decisions, not reflexes | M |
| 7 | Stuck timer counts world progress for patrols (§2.8) | **yes** (sim) | removes a cap on waiting — *after* the probe says it binds | S–M |
| 8 | Decision-grid sweep for the oracle's moves (§2.9) | no (new move set version) | `--from-route` becomes a solver; enables DAgger | M |
| 9 | ES/CEM polish of the champion's output layer | no | a local optimiser where the difficulty actually is (≈187 genes, well-conditioned) | M |
| 10 | 24-seed paired protocol as the bar for a default (§2.10) | — | stops the tree accumulating noise-driven defaults | S (per knob) |

Two things I would **not** do now, and the measurements say why: **widen the net** (h32 was worse on
l9 in my 8-seed arm and the repo's own capacity arm found no gain — the pack has no capacity problem),
and **add population** (pop 800 × 100 costs ~3.8× the episodes per solve of pop 200 × 400). The
bottleneck is what the loop is asked to prove and what it is allowed to see, not how many weights it
has.

---

## 4. What I would do first, concretely

1. **Two days: make "solved" mean something** (§2.1, §2.2). Add `solved` to `EpisodeResult`, use it in
   `wins`/`--stop-on-win`/panel, add the held-out validation set for records, and re-run
   `evo/tests/difficulty.rs` + the difficulty table. No version bump, no retrain, and every later
   measurement gets better.
2. **One day: fix the two scale bugs** (§2.3, §2.4). `fine_sigma`, archive children at the fine step,
   then the 24-seed arms on `l9` and `scratch-spike-tight`.
3. **Two days: the hot path** (§2.5). Lane-major buffer, f32x8 forward pass behind a feature test,
   reference-implementation assertion, and a benchmark arm in `bench` so the number is recorded rather
   than remembered. Then spend the win on §2.6's thinking dial rather than on a wider net.
4. **The next content pass decides the rest.** The repo's own conclusion is that the pack is the
   ceiling, and the two observations that would exercise the machinery — a turret timing window and a
   mover that has to be waited for — are exactly what §2.7's slots are for. Author those levels and
   the sensor bundle pays for its retrain; skip them and the slots are speculative.

### What has landed since this was written

Items 1–3 are implemented, and item 4's scalar half is: the forward pass is blocked four rows at a
time and measured **2.5× faster at bit-identical results** (2.90 → 1.15 cycles/MAC), which is the
non-SIMD answer to §2.5; the lane-per-genome version is still the way to a wider net, not the way to
the speedup this pass was after. `FINDINGS.md` records every measurement:

* **Solved means every trial** (`EpisodeResult::solved`, `GenerationReport::solved`, the panel's
  strict line, `bench`, `--stop-on-win`). On its own this did not make the champion reliable: four
  seeds of `chain` still saved champions that won 22 of 48 fresh episodes, because the record was
  still a maximum over draws.
* **Validation was not the fix either, the first time.** Four fixed episodes compared on *means*
  saved a champion that died on its own recorded episode while a solver was in the generation. Twelve
  episodes compared on solve count first, plus the rule that a solved record cannot be displaced by
  an unsolved one, fixed the artifact but not its reliability.
* **The strict stop is the fix.** `train --stop-on-win --stop-on-validated` — stop when the record
  has won every validation episode — moved the same four seeds from **22 / 48** fresh-episode wins to
  **48 / 48**, at a median cost of 23 generations. `bench --stop-on-validated` records the protocol;
  arms are not comparable across it.
* **Fine sigma and the archive.** `fine_sigma_scale` (default 0.25) is a separate step from the
  coarse sigma, and archive children use it. Archive-on with local children solved 6 of 8 seeds
  against 4 of 8 without (paired 6–0, p = 0.031, medians 147 against 238) — the recorded loss was a
  coarse-children result. The fine step's own contribution was **not** established at eight seeds
  (p = 0.69 inside the archive, p = 1.0 on fine children alone), and is kept on mechanism rather than
  on that arm.

---

## 5. What I did not verify

* **No paired 24-seed arm for anything in §2.3–§2.5.** My arms are 6–8 seeds on one batch. They are
  directions with a mechanism behind them, not established sizes — §2.10 applies to this document too.
* **Speedup of lane-per-genome SIMD is projected, not measured.** What is measured, against a
  reference loop in the same process, is the scalar inner loop (2.9 cycles/MAC one row at a time,
  1.15 with four rows blocked — 2.5×, bit-identical) and the net's share of a generation (≈ 9 of
  19 ms). A lane-per-genome prototype should be measured before the wider-net estimate is quoted as
  a result.
* **The patience probe (§2.8) was not run.** The 600-vs-1220 arithmetic is exact; whether every patrol
  phase actually requires a full-round-trip wait is not, and my solve results suggest most do not.
* **The ES/CEM-polish row of §3 is untested.** The existing ES arm is untuned by its own admission
  (σ = 0.05, lr = 0.01, λ = 200 in 1 803 dimensions); a *polish* on the 187-parameter output layer is
  a different, much better-conditioned experiment.
* **The tree has since changed** (see §4, "What has landed since this was written"): the solve
  signal, validation, the fine step, the blocked dense layer and the strict stop are in, with the
  measurements in `FINDINGS.md`. This section's caveats are about the *review's own* arms, which are
  still 6–8 seeds on one batch and were not re-run as 24-seed pairs. Review runs wrote generated files
  (`champions/*.json`, git-ignored; the last run on each level is whoever ran last), and the temporary
  benchmark examples were removed. `cargo test --workspace` is green: 687 run, 686 passed, 0 failed, 1 ignored.

---

## Appendix — commands and raw numbers

```bash
# baseline
cargo test --workspace            # 680 run, 679 passed, 0 failed, 1 ignored

# throughput (this machine: i5-9400, 6C/6T)
./target/release/train-cli train --level levels/l9-wait-and-climb.json \
    --generations 20 --pop 200 --seed 3 --quiet     # 0.019 s/gen, 10 363 pop-episodes/s
./target/release/train-cli train --level levels/l9-wait-and-climb.json \
    --generations 30 --pop 200 --seed 3 --quiet --hidden 64   # 0.046 s/gen
#   ... --feedback 8  0.019   |  --no-auto-trials 0.010  |  --action-repeat 2  0.025

# fine children, 8 seeds, l9, 200 generations, --stop-on-win
#   shipped      81 28 38 104 46 38 51 63      median 44.5
#   fine-0       89 41 82  78 44 51 51 53      median 51.0
#   feedback-8   88 33 50  33 60 16 91 41      median 45.5
#   hidden-32    48 67 50  97 35 59 119 36     median 53.0

# breadth vs depth, same total episodes, l9
#   pop 200 x 400 gens: solved 81 28 38 104 46 38       (6 seeds)
#   pop 800 x 100 gens: solved 52 36 45  86 52 44       (6 seeds)

# champion reliability: train --stop-on-win, then score on 4 fresh master seeds x 3 trials
#   l9 seeds 1/5/9  -> 6/12, 12/12, 6/12
#   l3 seeds 1/5/9  -> 6/12,  6/12, 6/12

# inner loop (temporary example, removed)
#   1 accumulator 1687 ns / 1600 MAC (3.69 cycles/MAC)
#   4 accumulators 850 ns
#   8 accumulators 801 ns
```

**Where to look in the code.** `Trainer::advance` ([ga.rs:1705](crates/evo/src/ga.rs#L1705)) is the
whole loop in one function; `Trainer::breed` ([ga.rs:2052](crates/evo/src/ga.rs#L2052)) is the four
kinds of slot; `SuiteTally` ([ga.rs:914](crates/evo/src/ga.rs#L914)) is where the trials are actually
folded and where §2.1's `solved` belongs; `observe_scene`
([sensors.rs](crates/sim/src/sensors.rs)) is the observation in one pass.
