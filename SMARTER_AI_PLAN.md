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

## Item 6: A standing generalist population

**Problem.** Every level starts evolution from near scratch, with at best a
clone of that level's own route. Nothing the AI learns on one level carries
to the next, so a level a player draws this minute is approached by a
population that has never seen a spike, a deck or a patrol, however many
hundreds of levels it has solved before.

**Implementation.**

1. Keep one population permanently trained across the level pack, using
   the suite training that already exists (the `ga_suite` test path and
   the suite seeding in the library), rotating levels so no single level
   dominates.
2. When a run starts on any level, seed a fixed share of generation zero
   from the generalist population's current elites, alongside the existing
   route clone and from-plan children. The share is a flag, default off
   until gated, like every other seeding source.
3. Refresh the generalist population between pack runs from the champions
   the pack produced, so the transfer runs both ways.

**Performance impact.** None at runtime: it is a seeding source, and the
generalist's own training happens offline between runs, not inside a run.

**Gate.** On three pack levels and one battery level, generations to
validated win against the current seeding mix, 24 seeds paired. The
generalist share must help or at worst be neutral everywhere; a level it
hurts loses it.

**Kill criterion.** If generation zero from the generalist is not better
than generation zero from the level's own history anywhere, transfer is a
story, not a mechanism: drop it.

**Effort.** M. Mostly orchestration around suite training that exists.

## Item 7: Learn from the player's own playthroughs

**Problem.** The strongest teaching signal available, a human completing
the level, is currently thrown away. The playtest already simulates every
tick of a human run through the same input path the AI uses, but no trace
is kept.

**Implementation.**

1. Record playtest runs: the per-tick actions of a human play session,
   stored beside the level like routes and champions are, with the usual
   header fields (level hash, physics version).
2. Feed a completed human run to the existing imitation code
   (`crates/evo/src/imitate.rs`) exactly as a planner route is fed: fit
   clones to the recorded decisions at the states the human actually
   visited, inject them as ordinary children.
3. A human run that used a technique the planner does not know is worth
   more than a planner route, not less: do not filter demonstrations
   through planner agreement.

**Performance impact.** None. Demonstrations are data; the fitting cost
is the imitation fit that already exists.

**Gate.** On one level where the default run stalls (`l11-the-elements`
or a battery level chosen for the purpose): generations to validated win
with a demonstration against the same run without one, paired seeds. A
single demonstration should visibly move the stall; that is the claim
being tested.

**Kill criterion.** If a fitted demonstration does not beat its control,
the bottleneck is the fitting, not the idea; record that and stop rather
than collecting more demos.

**Effort.** S for capture (the trace exists in the play loop), M for the
fitting path and its tests.

## Item 8: Phase randomisation in training

**Problem.** A policy can memorise a patrol's timetable instead of
learning to read the patrol. Training draws phases already, but narrowly
enough that a champion can be a phase specialist: brilliant on the phases
it trained against, helpless on held-out ones, which is exactly what
validation then catches, late and expensively.

**Implementation.**

1. Widen the phase draw for training episodes across patrol, mover and
   turret phases, up to the full phase space the validators sample from.
2. Keep the validation phase set (`VALIDATION_PHASES`) fixed and untouched:
   the honest exam does not move, the practice gets harder.
3. Report, per champion, the gap between training-phase and
   held-out-phase performance as a first-class number in the difficulty
   table, so memorisation is visible rather than anecdotal.

**Performance impact.** None: same episodes, wider draw.

**Gate.** The train/validation gap on the pack shrinks without
generations-to-win regressing on any level, 24 seeds paired. Levels with
phase-independent solutions should be unaffected, and the table will show
it if they are not.

**Kill criterion.** If the gap does not shrink, the memorisation theory is
wrong for this game and the wider draw is just noise added to training:
revert to the narrow draw and record the negative result.

**Effort.** S. The machinery (common episodes, phase draws) exists; this
is ranges and reporting.

## Item 9: Fuse the leg specialists

**Problem.** The segment campaign (WP4) produces a specialist clone per
leg of a hard level, then trains legs one at a time. The obvious untried
step is breeding the specialists into each other: a whole-level genome
whose opening legs descend from one specialist and whose closing legs
descend from another. Crossover between two genomes stuck at the same
wall mixes two failures; crossover between specialists mixes two
successes.

**Implementation.**

1. After a campaign run has produced its per-leg clones, seed a
   whole-level population with pairwise crosses of distinct leg
   specialists as a named seeding share, alongside the best whole-level
   genomes the campaign already keeps.
2. Reuse the shipped crossover operator unchanged; the experiment is about
   who the parents are, not a new operator.
3. Compare against the campaign's existing continuation (train the best
   single chain further) at equal total episodes.

**Performance impact.** None per episode; the claim is fewer episodes.

**Gate.** On the two levels the campaign exists for (`l11-the-elements`,
`l10-the-gauntlet-ii`): episodes to validated win, fused seeding against
the current campaign continuation.

**Kill criterion.** If fusion does not beat continuation, segment
specialists do not compose, which is itself worth knowing and recording:
the campaign keeps its current shape.

**Effort.** S. Parents, seeding and crossover all exist.

## Item 10: A committee champion

**Problem.** The window shows one genome: the replay, the title-screen
demo and the saved champion are a single point of failure. Its mistakes
are idiosyncratic; five top genomes rarely make the same mistake on the
same tick.

**Implementation.**

1. Add an ensemble policy for display and replay contexts: the top k
   genomes of a finished run (k = 5) each evaluate the observation; the
   action is the majority vote, ties broken by the leaderboard order, so
   the committee is deterministic.
2. Keep the single champion as the recorded artifact and the thing gates
   measure. The committee is a presentation of the run's result, and it
   must never be smuggled into a difficulty-table row as if it were a
   genome.
3. Validate the committee the same way a champion is validated: it must
   complete the level on the validation phases before the window may
   show it as the level's solver.

**Performance impact.** Training: none. Display: k forward passes for one
agent, negligible next to a frame of rendering.

**Gate.** Committee against its own best member on the validation phases
across the pack: completion and clock. The committee ships only if it is
never worse than its best member on completion.

**Kill criterion.** If the committee ties or loses, one genome was already
carrying the skill and the votes were noise: keep the single champion.

**Effort.** S to M. The forward pass exists; the work is the vote, the
determinism test and the validation wiring.

## Item 11: Let the technique archive select

**Problem.** The technique archive catalogues what genomes did (stomps,
wall jumps, springs, airtime) but only for presentation. Scalar fitness
routinely deletes a weird skill that a later wall needed, because the
genome carrying it was mediocre everywhere else. Diversity that is only
described is diversity that evolution cannot use.

**Implementation.**

1. Give the archive cells a breeding role: each cell (a technique band)
   keeps its best occupant, and a fixed share of each generation's
   parents is drawn from cells rather than from the leaderboard, in the
   style of quality-diversity archives (MAP-Elites).
2. This is a selection mechanism only, like the plan bonus: fitness does
   not change, no version moves, and every child still earns its record
   the ordinary way.
3. The cell share is a flag, default off until gated, and reported per
   run so its effect is separable from Item 1's arms.

**Performance impact.** None per episode; the claim is better use of the
same episodes on levels where a rare technique is the key.

**Gate.** On `l11-the-elements` and one technique-gated battery level:
generations to validated win against the current selection, 24 seeds
paired, plus a diversity measure (occupied cells over time) to confirm
the mechanism did what it claims before crediting it with any win.

**Kill criterion.** If occupied-cell diversity rises but solve times do
not, the archive is a museum: keep it for presentation, out of breeding.

**Effort.** M. The archive exists; the selection wiring and the diversity
reporting are the work.

## Item 12: Let the policy choose its commitment length

**Problem.** Decisions happen every 4 ticks, always (DECIDE_EVERY). On a
long straight that wastes three decisions in four; at a timed jump, four
ticks is coarse. The cadence is a compromise the policy never gets to
negotiate.

**Implementation.**

1. Add one output head: alongside the action, the net chooses a
   commitment of 1, 2, 4 or 8 ticks for which the action repeats before
   the next decision. At 1 the policy has finer control than today; at 8
   it skips decisions it does not need.
2. This is a genome shape change like Item 3's: it rides the same
   migration machinery, with the new head's weights zero-initialised so
   the initial behaviour reproduces today's fixed cadence as closely as
   the head's encoding allows, and it ships only inside Item 3's retrain
   window rather than forcing a second one.
3. The interpreter and the replay format treat the commitment as part of
   the action stream, so a saved run replays exactly.

**Performance impact.** Potentially negative cost: fewer decisions on
easy stretches means fewer forward passes per episode. The head itself is
one output neuron.

**Gate.** The Item 3 difficulty table, compared arm against arm: v14
senses with fixed cadence against v14 senses with chosen cadence. Solve
times must not regress, and decisions per episode should fall measurably
on open levels.

**Kill criterion.** If the policy collapses to always choosing one
cadence, the head is dead weight and the shape stays at Item 3's.

**Effort.** M, and deliberately folded into Item 3's window.

## Item 13: Grow the net on a plateau, function-preserving

**Problem.** The standing answer to "make the net bigger" is no, because
capacity is not the measured constraint on any level so far. But "not
measured anywhere yet" is not "never": the first plan already allows
spending Item 2's headroom on width when a level fails specifically on
capacity. Today there is no mechanism to do that mid-run, only a bigger
default for everyone forever.

**Implementation.**

1. Add a growth operator using the same trick as Item 3's migration: when
   a run's record has not improved for the plateau window (shared with
   Item 4's trigger), insert a small number of hidden units whose
   outgoing weights are zero. The policy is bit-identical at the moment
   of growth; evolution gains capacity it did not have and pays for it
   only from then on.
2. Growth is per-run and recorded in the run's output: the champion
   carries the shape it grew to, and autoshape already knows how to read
   mixed shapes.
3. Cap growth (two events per run, a fixed unit budget) so a stuck run
   cannot inflate itself indefinitely instead of solving.

**Performance impact.** Zero until a growth event fires; afterwards the
run pays for a modestly larger net, which is why this item stands behind
Item 2 in the sequence.

**Gate.** On a plateau case (the resumed l11 champion clock test from
Item 4): growth against no-growth at equal episodes. The clock must move
in the growth arm where it does not in the control.

**Kill criterion.** If grown capacity does not move the plateau it was
grown for, capacity was never the constraint there either: remove the
operator and let the "no bigger net" rule stand unqualified.

**Effort.** M. The migration proof pattern from Item 3 is reused; the
trigger and the caps are new.

## Sequencing

1. **Item 1** first: it is evidence, not code, and it may change what the
   defaults already are before anything new is built.
2. **Item 2** second: it banks the performance every later item spends.
3. **Item 5** third: small, pure saving, and it makes every later gate
   cheaper to run.
4. **Item 3** fourth: the one deliberate retrain, taken only with Item 2's
   headroom banked and the migration proof green.
5. **Item 4** last of the first tier: the largest build, aimed at the
   plateau the other items do not touch.

The remaining items need no order against each other and can be picked up
opportunistically, with two exceptions: Item 12 rides Item 3's retrain
window, and Item 13 stands behind Item 2's banked headroom and shares
Item 4's plateau trigger. Items 6 to 11 are independent, mostly cheap,
and each is gated on its own, so a failure in one says nothing about the
others.

## What this plan deliberately does not do

- **No GPU evaluation.** The net is tiny, the sim is the cost, and batching
  across genomes on a GPU would trade determinism and latency for nothing.
- **No bigger default net.** Capacity is not the binding constraint on any
  measured level; perception (Item 3) and credit assignment (Item 4) are.
  The one exception is Item 13: capacity a run grows for itself, on a
  plateau, with the plateau as the evidence, never a larger default
  handed to every level in advance.
- **No fitness version move.** Nothing here needs one: the plan bonus stays
  a selection mechanism, lessons enter through breeding, and early
  termination is defined to be score-identical. The only version that
  moves is sensors, once, in Item 3.
