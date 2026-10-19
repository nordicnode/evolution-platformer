# Smarter, not slower: the next AI improvements

October 2026. Companion to [AI_IMPROVEMENT_PLAN.md](AI_IMPROVEMENT_PLAN.md), and
narrower than it: every item here has to make the AI smarter **without lowering
performance** on the baseline machine (i5-9400, 16 GB RAM, no GPU help: the net
is 1,931 genes and the sim is CPU-bound, so the GPU gains nothing and costs
determinism).

Two rules govern the whole document:

1. **The performance budget is a gate, not a hope.** The reference figure is
   today's generation cost, about 19 ms at pop 200 on `l9-wait-and-climb`.
   No item lands if its benchmark generation is slower than the one before it
   on the same machine class, unless an earlier item in this sequence has
   already banked more speed than the item spends.
2. **Bit-identical where claimed.** Changes that are sold as "the same AI,
   faster" or "the same champion, wider senses" have to prove it: identical
   episode trajectories and state hashes, not just similar scores.

What is already in, and therefore not proposed again: rayon-parallel
population evaluation, mirrored CEM polish (`crates/evo/src/polish.rs`), an
OpenAI-ES baseline arm (`crates/evo/src/es.rs`), islands
(`crates/evo/src/islands.rs`), the Go-Explore style snapshot archive
(`crates/evo/src/snapshots.rs`), plan-guided selection (WP5), segment
distillation (WP4), common episodes and raced trials.

## Item 1: Turn on what is already built (the gates)

**Problem.** Three landed features sit behind flags at conservative defaults
because their measurement gates were never run: jump-timing children
(`GaConfig::jump_timing_share` ships at 0.0), the plan-proximity selection
bonus (`--plan-bonus`, terminal-only), and islands (built, smoke-tested, not
the default). The code exists; only the evidence is missing.

**Implementation.**

1. Run the plan's standing protocol for each arm: 24 seeds, paired against
   the current default at an equal episode budget, using the `bench`
   command's protocol/arm machinery so the runs are recorded and comparable.
   - Jump-timing: `--jump-timing-share 0.5` on `l2-first-gap` and one
     timing-heavy level (`l4-rhythm`).
   - Plan bonus: `--plan-bonus` at the shipped weight on
     `l11-the-elements` and `bat-pit-bait`, including the decay check:
     champions re-scored under pure fitness v9 must not collapse.
   - Islands: the equal-episode comparison against single-population on
     `l9-wait-and-climb` and `l11-the-elements`.
2. Record every result in FINDINGS.md, passing or failing.
3. Flip the default only for arms that pass. A failed arm keeps its flag and
   gains a recorded reason.

**Performance impact.** None at runtime for the first two (same episodes,
same net). Islands spend the same episodes across cores rather than in one
population.

**Gate.** Per arm: median generations-to-validated-win improves, and no pack
level regresses beyond the paired sign test's noise floor.

**Kill criterion.** An arm that does not move its levels after the 24-seed run
stays off, permanently, with the numbers in FINDINGS.md as the reason.

**Effort.** Machine time and recording, roughly a day of wall clock, no new
code beyond defaults.

## Item 2: The lane-per-genome SIMD forward pass

**Problem.** The forward pass (`Genome::forward` in
`crates/evo/src/genome.rs`) is about half the cost of a generation. It is run
one genome at a time, so the same weight loads are repeated for every genome
in the population while the CPU's vector lanes sit idle. This is the item
that pays for Items 3 and 4.

**Implementation.**

1. Add a batched entry point beside `Genome::forward` that evaluates a block
   of genomes (4 or 8, matching the vector width) against the same layer
   weights layout: for each output neuron, load the weight row once and
   apply it to every genome in the block, accumulating in the same
   per-neuron order the scalar pass uses.
2. Keep the arithmetic order per output identical to the scalar pass, so
   results are bit-identical, not approximately equal. No fast-math, no
   reordered summation.
3. Route population evaluation (`crates/evo/src/ga.rs`) through the batched
   path in blocks; keep the scalar path for single-genome callers (champion
   replay, the window) and as the `wasm32` fallback.
4. Benchmark before and after on `l9-wait-and-climb` at pop 200, and on a
   feedback-16 shape, since the recurrent state is part of the scratch.

**Tests.**

- A property test: random nets and random observations, batched output
  bitwise equal to scalar output, across shapes including feedback nets.
- The existing replay guarantees stand in for the integration test: a saved
  champion replays to the same frames, progress and outcome as before.

**Performance impact.** Projected 4 to 6x on the forward pass (the figure
recorded in the AI review), so generation cost should fall meaningfully even
after Items 3 and 4 spend their share.

**Gate.** Measured, not projected: generation wall time on the reference
level improves by at least 25 percent, with the bit-identity tests green.

**Kill criterion.** If the measured gain is under 10 percent, the added
complexity is not worth keeping; revert to the scalar pass and record why.

**Effort.** M. The risk is all in step 2: bit-identity is a hard requirement,
not a stretch goal.

## Item 3: Sensors v14 by function-preserving growth

**Problem.** The policy is blind in four specific ways, all recorded in the
first plan: it cannot see how long a crumbling tile has left, how fast the
nearest threats are moving, enemies inside its own 9x9 patch, or a second
moving platform. Sensors v14 fixes all four (crumble fuse countdown, threat
velocities, `ENC_ENEMY` in the patch, second mover velocity and phase,
about 15 new inputs). It was implemented once and reverted: the trial
rebuilt the observation vector, every existing champion and warm start
effectively began as a newborn, and the warm-start gate on `l2-first-gap`
regressed (23 generations against random's 18).

**Implementation.**

1. Append the v14 inputs at the end of the observation vector in
   `crates/sim/src/sensors.rs`. Every v13 input keeps its index, so the
   meaning of existing weights is untouched. Bump `sensors::VERSION` to 14.
2. Add a shape migration in `crates/evo/src/genome.rs`: a v13 genome loaded
   into a v14 shape keeps its weights and gets **zero** weights for the new
   input columns. A zero-weighted input cannot change a decision, so the
   migrated genome is the same policy, provably.
3. Prove step 2 the way champion replay is already proven: the saved l11
   champion, migrated, replays to identical frames, best_x, progress and
   outcome.
4. Retrain the pack once (this is the plan's one perception retrain), then
   run the full difficulty table: 24 seeds, every pack level, plus the
   battery levels this item exists for (`bat-crumble-bridge`,
   `bat-double-threat`).

**Performance impact.** The forward pass grows by roughly the input growth
(about 15 percent per decision at the current shape). This item may only
land after Item 2 has banked more than that, per rule 1.

**Gate.** The v14 gate as originally written: no pack level regresses, and
the crumble and double-threat battery levels move from unsolved-by-default
to solved. The l2 warm-start regression that killed the first attempt is
run explicitly and must not recur: with function-preserving growth, the
warm arm starts from the v13 policy itself rather than from a newborn, so
the comparison that failed last time is now structurally incapable of
failing in the same way.

**Kill criterion.** If the battery levels do not move, v14 stays a branch:
the senses cost real cycles, and senses the search cannot use are a tax,
not an investment.

**Effort.** M for the sensor and migration work; the retrain and difficulty
table are machine time.

## Item 4: Teach during the run, not just before it

**Problem.** The planner currently teaches at the start and then leaves:
the plan bonus decays to zero, and cloning happens as a front-loaded
`--from-plan` or as a separate `campaign` command. The documented failure
this leaves behind is the plateau: a saved l11 champion resumed for 2,000
generations without its clock improving, because the population only ever
sees states its current policies reach, and its current policies never
reach the states the better route goes through.

**Implementation.**

1. **Mid-run DAgger lessons.** Every `--lesson-every` generations (default
   off until gated), take the run's record genome, play its episode, and
   collect the states it actually visited. Query the planner
   (`crates/evo/src/navigate.rs` and the timed plan machinery) for the
   action it would take in those states, fit a small number of clones to
   those decisions with the existing imitation code
   (`crates/evo/src/imitate.rs`), and inject the clones as ordinary
   children. Lessons target the states the run really reaches, which is
   the known weakness of cloning a route the population never walks.
   Clones enter through breeding, so no episode that did not start at
   spawn can touch the record.
2. **Plateau-triggered CEM phases.** When the record has not improved for
   `--plateau-window` generations, spend a bounded slice of the run's own
   episode budget on a mirrored CEM phase over the full net, generalised
   from `crates/evo/src/polish.rs` (which today polishes only the output
   layer, and only after a run). A phase's winner is adopted only if it
   certifies on held-out seeds, the same certification the polish command
   already uses.
3. Before building step 2, run the comparison the ES baseline arm exists
   for: `crates/evo/src/es.rs` against the GA at equal episodes on the two
   plateau cases (the l11 clock, `bat-deck-phase`). If the ES arm does not
   beat the GA there, step 2 is built on the GA's own mutation instead, or
   not built.

**Performance impact.** Both halves spend episodes from the run's existing
budget rather than adding new ones; the cost is the episodes they redirect,
which is why the gates are equal-budget comparisons.

**Gate.** On `l11-the-elements`: generations-to-validated-win against the
current default, 24 seeds paired. On the resumed-champion clock test: the
clock must move within 2,000 generations where the unmodified run did not.

**Kill criterion.** Lessons that do not beat their equal-budget control are
a distraction dressed as teaching: remove the trigger, keep the numbers.

**Effort.** L. Step 1 is mostly wiring existing parts; step 2's
generalisation of the polish code is the real work.

## Item 5: Stop paying for episodes that have already failed

**Problem.** Every genome in a generation is played for its full episode or
until it dies, including genomes that stopped making progress hundreds of
ticks ago and are standing still against a wall. The fitness code already
knows what "stuck" means (the stuck timer in `crates/evo/src/fitness.rs`);
the evaluation loop just does not act on it.

**Implementation.**

1. During evaluation, end an episode early when its own stuck timer has
   run out and its remaining possible progress cannot change its ranking
   band (it cannot catch the genome one rank above the selection cut).
   Score it with exactly the fitness its partial episode earns today, so
   early termination changes cost, never meaning.
2. Return the saved ticks to the same generation: either as extra trials
   for the genomes near the selection cut (where ranking errors actually
   happen), or as a larger effective population at the same wall clock.
   Pick one per experiment, not both, so the gate can attribute the gain.
3. The one hard rule: a terminated episode must score bit-identically to
   the same episode played out. A test pins this on a level with a known
   wall-stander.

**Performance impact.** Pure saving on levels with many hopeless agents,
which is most levels in early generations. This is the cheapest item in
the document and the only one whose benefit grows as levels get harder.

**Gate.** Same champions, same difficulty table rows, less wall time; or,
if the refund is spent on trials, a measured improvement in
generations-to-validated-win at equal wall time.

**Kill criterion.** If the measured saving is noise (under 5 percent of
generation time on the reference levels), drop it: an evaluation loop with
a special case has to earn its keep.

**Effort.** S to M.

## Sequencing

1. **Item 1** first: it is evidence, not code, and it may change what the
   defaults already are before anything new is built.
2. **Item 2** second: it banks the performance every later item spends.
3. **Item 5** third: small, pure saving, and it makes every later gate
   cheaper to run.
4. **Item 3** fourth: the one deliberate retrain, taken only with Item 2's
   headroom banked and the migration proof green.
5. **Item 4** last: the largest build, aimed at the plateau the other items
   do not touch.

## What this plan deliberately does not do

- **No GPU evaluation.** The net is tiny, the sim is the cost, and batching
  across genomes on a GPU would trade determinism and latency for nothing.
- **No bigger default net.** Capacity is not the binding constraint on any
  measured level; perception (Item 3) and credit assignment (Item 4) are.
  If a battery level ever fails specifically on capacity, the answer is
  Item 2's headroom spent on width, with that failure as the evidence.
- **No fitness version move.** Nothing here needs one: the plan bonus stays
  a selection mechanism, lessons enter through breeding, and early
  termination is defined to be score-identical. The only version that
  moves is sensors, once, in Item 3.
