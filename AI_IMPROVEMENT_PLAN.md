# AI improvement plan — October 2026

**Goal (owner's words):** improve the AI evolution/learning until it is smart,
sophisticated, able to learn and adapt, and able to complete any level the player
can create — exploring and exploiting the level and the game mechanics to the best
of its ability — while performing well on modest hardware.

**Baseline machine** (the "consumer hardware" bar for every number below, same as
the October 2026 AI review): Intel Core i5-9400, 6 cores / 6 threads, 2.9 GHz
base / 4.1 GHz turbo, 16 GiB RAM, GTX 1660 Super, AVX2 + FMA, no AVX-512.

**Relationship to the October 2026 AI review (AI_REVIEW.md, retired 2026-10-18
and preserved in git history; section citations below refer to it).** That review asked where the learning loop leaves
intelligence on the table, and its top items have since landed: "solved" means won
every trial, champions are validated on held-out episodes (48/48 fresh wins under
`--stop-on-validated`), the fine mutation scale is separate, the forward pass is
blocked and 2.5× faster bit-identically, sensors v13 gave the policy a clock, and
`sim::plan` is a model-based teacher. This plan does not repeat that roadmap. It
starts from what those changes exposed and aims at the owner's goal above.

**Status of this document:** plan only. No code in this plan has been implemented
or measured yet, except where a number is explicitly cited from AI_REVIEW.md or
FINDINGS.md. Every work package carries the measurement that would justify — or
kill — it.

---

## 0. Verdict

The search algorithm is not the binding constraint. The GA is well built and well
measured: a generation of 200 genomes on l9 costs ~19 ms on the baseline machine
(~10,363 population-episodes/s), and the game window sits idle for 95%+ of a
watched run. The project has compute headroom to spare; what it lacks is
sample efficiency and policy expressiveness.

The proof is the pack's frontier level, `l11-the-elements`:

- `sim::plan` finds a 240-block, 957-frame route through its three mover decks
  in ~3 s (AI_REVIEW §4, "What has landed").
- The behaviour clone of that route reaches 97% action agreement — and dies at
  x 30, before the first deck.
- 2,000 generations at pop 200 seeded from the plan reached x 62 of 68, no win.

**The system already knows how to beat the level. The learning loop cannot absorb
that knowledge, and the default policy cannot express the behaviour** (it is a
memoryless reflex: `feedback = 0`, so it cannot count, wait, or latch a decision,
while the level's crossings are timed). Everything in this plan follows from
those two facts, in that order.

### What "any level the player can create" can honestly mean

The tile and entity set contains no keys, doors, buttons or collectibles: every
level is reach-the-flag. Within that set, this plan defines the target as:

> **Any level that the reachability oracle (`reach.rs`) and the timed planner
> (`sim::plan`) can jointly prove solvable, the system completes** — by the
> evolved policy where it can learn it, and by the planner's own route as the
> solver of record where it cannot yet — and the editor states the verdict when
> the level is saved.

That turns an impossible promise into a checkable one, and WP6 makes the editor
say it out loud.

---

## 1. The loop as it stands (the parts this plan touches)

| Layer | Mechanism | Default | Where |
|---|---|---|---|
| Policy | fixed-topology MLP, 108 → 16 → 11, `tanh`, argmax, 1,931 genes | memory **off** (`feedback 0`) | `crates/evo/src/genome.rs` |
| Observation | 9×9 patch (radius 4) + 27 scalars, sensors v13 | nearest threat only; no fuse, no threat velocity | `crates/sim/src/sensors.rs` |
| Search | tournament GA, pop 200, uniform crossover, Gaussian mutation | σ 0.30, rate 0.05, 4 elites, 8 fine children | `crates/evo/src/ga.rs` |
| Diversity | MAP-Elites archive, 240 cells | **observational**: `archive_children 0`, `novelty_weight 0` | `crates/evo/src/ga.rs` |
| Fitness | progress + 32/hop + 16/checkpoint + 16/deck − 2/jump − 20/death + 10,000/win + speed | v9 | `crates/evo/src/fitness.rs` |
| Escape | stall → derived practice level, 40-gen detour, one crossing at a time | on | `crates/evo/src/curriculum.rs` |
| Teacher | `sim::plan` route → clone + DAgger → warm-start a quarter of gen 0 | `--from-plan`, once, at run start | `crates/evo/src/imitate.rs` |
| Planner | best-first search over full `Sim` snapshots at decision cadence | beam 64, depth 32, teacher/diagnostic only | `crates/sim/src/plan.rs` |

Known blind spots, evidenced in code (sensors.rs unless noted):

- Only the **nearest** threat is sensed, as position + a ±1 direction sign. No
  threat velocity (a shot's speed is unobservable), no second threat at all.
- Enemies never appear in the 9×9 patch: `ENC_ENEMY` is defined and never used.
- Cracked tiles show as a patch value but carry **no fuse countdown** — the
  remaining crumble time is unobservable, so "get off now" is unlearnable.
- Only the **nearest** mover (≤12 tiles) is sensed; goal/waypoint scalars
  saturate at 16 tiles, so half of a wide level reads identically.
- `genome.rs` contains `mutate_jump_timing`, a targeted operator for the jump
  boundary — and `Trainer::breed` never calls it. The search does not use the
  one operator built for its hardest micro-decision.

---

## 2. Ground rules for every package below

1. **Measurement protocol.** No default changes on fewer than 24 paired seeds
   (AI_REVIEW §2.10). Arms are recorded in FINDINGS.md with the protocol named;
   `bench --compare` already refuses cross-protocol comparisons — keep using it.
2. **Version-trio discipline.** Changes to physics, sensors or fitness move a
   version and refuse every existing champion. Version-moving packages are
   batched into as few retrain windows as the measurement protocol allows
   (Phase B and Phase D each move exactly one version; see §8).
3. **Determinism is a feature.** Everything stays seeded, thread-count
   independent and bit-reproducible. This rules out GPU policy evaluation
   (§6) and any library that hides its RNG.
4. **The fun gate still governs the window.** The product is the visible
   learning arc. Packages that change what a watched run looks like (WP3,
   WP6's solver ghost) are presentation decisions for the owner, flagged as
   such, not silent defaults.
5. **Kill criteria are stated up front.** Each package names the result that
   would retire it. A measured negative is a result, not a failure — this repo
   already runs on that rule.

---

## 3. Phase A — quick wins (days, no version moves, no retrain)

### WP1. Wire `mutate_jump_timing` into breeding

- **Problem.** The targeted jump-timing operator exists and is unused; generic
  mutation moves ~97 genes per coarse child, which is a blunt way to adjust the
  one boundary (when to press jump) that decides most crossings.
- **Change.** `Trainer::breed` (`ga.rs`): give the 8 fine children and the
  practice detour's timing seeds (`curriculum.rs`, `PRACTICE_TIMING_SEEDS`)
  a share of jump-timing children — start at half the fine slots — bred from
  the generation's best like other fine children.
- **Version impact:** none (search only).
- **Gate.** 24-seed paired arm on l9-wait-and-climb and a tight-gap level:
  median generations-to-first-win vs. current fine children. Retire if the
  paired test is null at 24 seeds.
- **Effort:** S (under a day including the arm).

### WP2. Auto-shape the brain from the level scan

- **Problem.** Memory is opt-in via `--feedback`, and the shipped pack solves
  without it, so the default run is a reflex even on levels whose solutions
  require waiting, counting or latching (turrets period 120 ticks, lava vents
  150, mover cycles, crumble fuses). Sensors v13 supplies a clock and an event
  ETA that a memoryless net can barely act on. Likewise `rank_pessimism`
  defaults to 0 on stochastic (walker) levels where AI_REVIEW §2.2 showed
  luck-of-the-phase distorts ranking.
- **Change.** At run start, scan the level (the loaders already classify every
  mechanic): if it contains turrets, movers, cracked tiles or flyers, default
  `feedback = 16` and a non-zero `rank_pessimism`; log the choice and the
  reason in the run header so it is visible, overridable, and testable. The
  level tells the trainer what brain it needs; the operator flag stays as an
  override.
- **Version impact:** none — shape is recorded per champion already; fitness
  and sensors are untouched.
- **Gate.** Paired arms on l9 (patrol timing) and l11: reflex-default vs.
  auto-shape. Also a no-regression arm on l1–l5, where memory must not hurt.
  Feedback at width 8 measured free in AI_REVIEW (19 ms/gen); re-measure at 16.
- **Effort:** S–M (1–2 days; the scan exists in pieces in `reach.rs` and the
  level loader).

### WP3. The thinking dial (window)

- **Problem.** AI_REVIEW §2.6, still open: the window builds
  `GaConfig::default()` and idles 95%+ of a watched run at 15–19 ms/gen.
  The in-game AI is the least capable configuration the project ships.
- **Change.** A panel control: generations computed per displayed generation
  (1×/4×/16×/64×), plus best-of-N restarts behind the same dial. Pure
  scheduling — the trainer already runs off-thread (`app.rs`).
- **Version impact:** none.
- **Gate.** Not a competence claim; verify frame budget only: at 64× the
  window must still animate at watch speed on the baseline machine
  (64 × ~19 ms ≈ 1.2 s per displayed generation — inside the existing
  0.6–5 s animation window).
- **Effort:** M (2–3 days, mostly panel work). **Owner decision:** dial
  presentation and default.

---

## 4. Phase B — put the planner inside the learning loop (the core phase)

This is the phase the whole plan is for. The planner is currently a teacher who
visits once, at generation 0, hands over a whole-route clone that provably
cannot survive contact with the policy's cadence, and leaves. The fix is to
make planning a recurring part of training, in segments small enough to learn.

### WP4. Segment distillation instead of whole-route cloning

- **Problem.** A whole-route clone fails at the first segment it executes
  imperfectly (dies at x 30 of 68). Errors compound; DAgger over the whole
  route labels states the student reaches, but the fit is still one global
  problem. The existing practice-detour already proves the unit that works:
  **one crossing, judged with the climb** — it just only ever runs one hop,
  reactively, after a stall.
- **Change.** When a plan exists for a level (or is computed at run start):
  1. Split the plan's route at the route graph's stands into segments
     (`reach::GoalRoutes` rungs are the natural boundaries).
  2. Train segment *k* as a practice level (spawn at the segment's entry
     stand, flag at its exit stand), seeded from the plan's demonstration
     for that segment only — cloned at the policy's block cadence, which
     `imitate::from_plan` already does correctly.
  3. Chain: a genome graduates to segment *k+1* practice only via the suite
     mechanism that already exists (`[practice, main]` ranked worst-of-two),
     so earlier segments are not bred away while later ones are learned.
  4. On a stall in the main run, re-plan **from the stall snapshot**
     (`plan::solve_from` already supports mid-level starts) and distill the
     *next* segment from there, instead of only deriving a practice level
     from the route graph.
- **Version impact:** none (trainer/curriculum behaviour; the demonstrations
  and segment levels are run artifacts).
- **Gate — the plan's headline test.** l11-the-elements: an evolved champion,
  strict-solved and validated under the existing protocol, within a stated
  budget (propose: median ≤ 500 generations at pop 200 across 12 seeds,
  where the current best effort is 2,000 generations with no win at all).
  Secondary: no regression on the l1–l9 difficulty table.
- **Effort:** L (the largest package here; ~1–2 weeks). It is mostly
  composition of parts that exist: `plan`, `imitate`, `curriculum`, suites.
- **Kill criterion.** If segment chaining solves each segment in isolation
  but the chained suite cannot hold them together at 24 seeds, the bottleneck
  is retention/interference, not distillation — stop and go to WP7/WP8 with
  that finding recorded.

### WP5. Planner-guided fitness (dense credit where the geodesic lies)

- **Problem.** Fitness v9's progress is geodesic: route hops closed, monotone.
  Where the route graph is degenerate — switchbacks, go-left-first, wait-for-
  the-deck — shaping falls back toward straight-line distance, which the v5
  notes themselves show points the wrong way; and there is no credit at all
  for *being in phase*: waiting correctly, riding a deck, standing in the
  right place at the right tick all score zero until the hop closes.
- **Change.** Fitness v10 adds one term: a **plan-proximity bonus** — the
  fraction of the plan's visited stands (or, cheaper, distance closed along
  the plan trajectory's waypoint sequence) the episode achieved, weighted
  small against the 10,000 win bonus and **decaying to zero over the run**
  (e.g. linearly over the first half of the generation budget), so the final
  selection pressure is the unshaped objective and the champion is not a
  plan-mimic that cannot improvise. Where no plan exists, the term is absent
  and v10 reduces to v9.
- **Version impact:** **fitness v9 → v10.** Batch with nothing else; the pack
  retrains once for Phase B (§8).
- **Gate.** 24-seed paired arms, plan-bonus vs. v9, on l11 and on one
  switchback-style level from the battery (§9). Decayed-to-zero must be
  verified: final champions re-scored under pure v9 must not collapse.
- **Effort:** M (3–4 days; the plan trajectory is already computed for WP4).
- **Risk.** Reward hacking the proximity term (hovering near the plan path).
  The decay and the small weight are the mitigation; the gate's v9 re-score
  is the check.

### WP6. Solver of record, anytime planner, and the editor verdict

- **Problem.** Even with WP4/WP5, some level will exceed the evolution budget.
  Today that level just never gets beaten, and the player is not told whether
  it is beatable at all.
- **Change.**
  1. **Anytime, parallel planner.** `plan.rs` beam search widened adaptively
     (beam/depth grow while time remains in a stated budget, e.g. 10 s),
     expansions parallelised across the 6 cores with results folded in the
     established deterministic order. One-off per level, cached by content
     hash next to the existing route cache (`route_cache.rs` discipline).
  2. **Solver ghost.** If evolution exhausts its budget on a
     planner-solvable level, the plan route is offered as the level's solver
     replay (the replay machinery already ghosts champions; a plan route is
     the same artifact shape). Evolution continues from it via WP4 — the
     ghost is a floor, not a surrender.
  3. **Editor verdict.** On save, run reach + plan: the level card states
     *solvable by route / solvable by timed plan (difficulty estimate:
     hops, tightest timing window) / no route found*. This is the honest
     form of the owner's goal, surfaced where levels are made.
- **Version impact:** none (planner internals and presentation; plan routes
  are not champions).
- **Gate.** Planner budget test: every pack level plus the battery (§9)
  either solved inside the budget or reported unsolved-by-planner, with
  timings recorded in FINDINGS. Determinism test: parallel planner output
  bit-identical across thread counts, same standard as the GA's existing
  test.
- **Effort:** M–L (parallelising the beam carefully is the bulk).
- **Owner decision:** whether the solver ghost appears automatically or only
  on request — it changes what "the AI beat my level" means in the window.

---

## 5. Phase C — exploration that survives hard levels

### WP7. Islands instead of one population

- **Problem.** The diversity machinery measured as harmful — archive trickle
  10/12 → 7/12 on the hardest level, archive children and novelty defaulted
  off. The likely mechanism is contamination: niche genomes bred into the
  main line at the coarse scale destroy what the niche was for. The correct
  conclusion is not "diversity doesn't pay" but "diversity doesn't pay
  *in one panmictic population*".
- **Change.** 4–6 islands (one per core on the baseline machine), each pop
  100–200, each with its own settings drawn from a fixed spread: sigma
  (0.2 / 0.3 / 0.45), feedback (0 / 16), novelty/archive breeding (off / on,
  island-local), pessimism (0 / on). Ring migration of each island's best
  every 25 generations, migrants entering as fine children, never as
  replacements for elites. The MAP-Elites archive becomes per-island and is
  bred from inside its island only. Single-population mode remains the
  default until the gate passes.
- **Version impact:** none.
- **Compute.** 6 islands × pop 200 × ~19 ms/gen, one island per core ≈ the
  same wall-clock per generation as today, ~6× the episodes. On the
  baseline this is affordable precisely because a generation is 19 ms, not
  19 s.
- **Gate.** 24-seed paired arms on the two hardest pack levels + two battery
  levels: islands vs. single pop at **equal episode budget** (not equal
  generations). This comparison must be episodes-matched or it proves
  nothing.
- **Effort:** M–L (trainer restructure; evaluation already parallelises
  per genome, islands add a scheduling layer).
- **Kill criterion.** No paired gain at equal episodes on at least two of
  four levels → keep single-pop, record the negative, retire.

### WP8. Snapshot archive: generalise the practice detour (Go-Explore shape)

- **Problem.** The stall → practice-level detour is a one-cell version of a
  known, stronger idea: return to a promising *state*, explore from there.
  Geodesic fitness cannot reward mapping a level, revisiting a stand at a
  different phase, or surviving past a deceptive pit to find the real route —
  the observation has no visited-area encoding and episodes always start at
  spawn, so a 60-tile level's late crossings are practised only by genomes
  that already solved everything before them.
- **Change.** Maintain an archive of `Sim` snapshots keyed by
  (stand, phase bucket, mover-phase bucket, crumble state) — the phase keys
  matter: the same stand at a different patrol phase is a different problem.
  A fraction of each generation's episodes (propose 1 in 8) start from an
  archived frontier snapshot instead of spawn, chosen by fewest-visits;
  snapshots are admitted when an episode reaches a new cell or a known cell
  with materially better remaining budget. Fitness for snapshot-started
  episodes is scored on progress *from that snapshot*, and such genomes only
  enter the record through the existing full-episode validation — the
  archive explores, it does not crown.
- **Version impact:** none to sim/sensors/fitness arithmetic (episode start
  state is a trainer concern; validation stays spawn-started).
- **Gate.** The deceptive battery levels (§9: pit-bait, backtrack) at 24
  seeds, episodes-matched, vs. the current detour chain. Also re-run the
  documented stall-chain protocol (24/24 with detour vs. 17 and 9 without)
  to confirm no regression in the mechanism this generalises.
- **Effort:** L (snapshot lifecycle + admission policy are new; `Sim`
  already snapshots for the planner, which is the existence proof).
- **Memory cost.** Snapshots are small (a level is ≤ a few hundred tiles of
  state); thousands of cells are megabytes. A non-issue at 16 GiB.

### WP9. CEM polish of the champion's output layer

- **Problem.** AI_REVIEW roadmap item 9, still untested: the shipped ES is a
  1,931-dimension baseline (λ=200 gradient signal ~√(λ/n) ≈ 0.32 — weak by
  construction). But the *output layer alone* is 16×11 + 11 ≈ 187 parameters,
  and the decisions that fail (jump timing, action boundaries) live there.
- **Change.** After a run's record is set (or on demand, `train --polish`):
  a small (μ, λ) CEM/ES over the champion's output-layer genes only, σ
  annealed, evaluated under the same fixed validation episodes that crown
  records. Hidden layers frozen. Any polished genome must re-pass full
  validation to replace the record.
- **Version impact:** none.
- **Gate.** Post-hoc test on existing champions: how many gain validated
  wins/speed after polish, at what episode cost. If the median champion
  gains nothing, retire — this is a cheap experiment, run it early in
  Phase C, not last.
- **Effort:** S–M (the ES machinery exists in `es.rs` to crib from).

---

## 6. Phase D — perception, in one retrain (sensors v14)

Bundle every observation change into a **single** sensors version bump, after
Phases B–C have stabilised, so the pack retrains once for perception rather
than once per idea. Proposed v14 contents, each traceable to a blind spot in §1:

1. **Crumble fuse countdown** for the tile under/near the body (ticks
   remaining / `CRUMBLE_TICKS`, clamped) — makes cracked bridges a decision.
2. **Top-3 threats, object-slotted:** for each of the three nearest threats,
   dx, dy **and vx, vy** (normalised as the body's own velocities are).
   Replaces the single-threat slots; fixed slot count keeps the vector
   fixed-size and the net fixed-topology.
3. **Enemies in the patch:** use the already-defined `ENC_ENEMY` for live
  threat cells inside the 9×9 patch, so spatial patterns (two walkers
  converging) are visible, not just the nearest one's vector.
4. **Second mover:** nearest mover slots duplicated for the second-nearest
   mover within range (position enters via patch already; add velocity +
   phase for mover #2).
5. **Feedback on by default** (`feedback = 16`) in the shipped shape,
   superseding WP2's heuristic if WP2's gate passed — the clock sensors
   have existed since v13; v14 is where the default policy gains the state
   to use them.

**Deliberately not included:** a bigger patch. The patch scan is part of the
per-decision sensor cost and radius 4 → 6 grows it 81 → 169 cells for
information the waypoint and object slots carry more cheaply. If a battery
level (§9) fails specifically on patch range, revisit with that evidence.

- **Version impact:** **sensors v13 → v14** (fitness unchanged in this
  phase). One pack retrain; FINDINGS records the new difficulty table.
- **Gate.** Full difficulty table re-run (24 seeds): no pack level regresses;
  the crumble and double-threat battery levels must move from
  unsolved-by-default to solved.
- **Effort:** M for the sensor work; the retrain and table are machine time.

### Performance budget (baseline i5-9400, all measured unless marked)

| Item | Cost | Note |
|---|---|---|
| Generation, pop 200, l9, today | ~19 ms | AI_REVIEW appendix |
| Forward pass share | ~half of a generation | blocked dense: 1.15 cycles/MAC, bit-identical |
| Islands (WP7), 6 × pop 200 | ~19 ms wall/gen (projected) | one island per core; ~6× episodes at equal wall-clock |
| Planner, one level | ~3 s (l11, beam 64) | WP6 anytime budget caps at 10 s; cached by level hash |
| Snapshot archive (WP8) | 1-in-8 episodes redirected | no new episodes; admission bookkeeping is trivial vs. sim cost |
| Sensors v14 (Phase D) | +~15 inputs, same patch | forward pass grows ~15%; still ~1 µs/decision class |
| GPU | **unused, deliberately** | a 1,931-gene net + CPU sim gains nothing from a 1660 Super; batching across genomes would cost determinism and latency. The GPU stays the renderer's. |
| RAM | « 16 GiB | populations, archives and snapshots are megabytes in total |

If wider or recurrent nets ever do become the bottleneck, the enabling step
is the lane-per-genome SIMD forward pass (AI_REVIEW §2.5, projected 4–6×,
bit-identical) — before any width increase, and only on evidence a battery
level needs the capacity.

---

## 7. Phase E — the wow phase: discovery, mastery, showmanship

Phases A–D make the AI *competent*. This phase (added at the owner's request,
2026-10-18) aims at a different reaction: **"the AI learned how to do THAT?
I didn't even know that was possible!"** That reaction is a design target, and
the current system suppresses it in four specific ways:

1. **It stops at the first win.** `--stop-on-win`, a 10,000-point win bonus
   and a speed bonus worth at most 200 of it: the first sloppy solution ends
   the run. Nothing is ever optimised, refined, or made strange.
2. **It taxes experimentation.** −2 per jump and −20 per death teach the
   population that the intended-looking route is the good route.
3. **Its diversity archive cannot see technique.** MAP-Elites cells are
   (progress × checkpoints × finish time). A genome that stomps five walkers
   to cross a pit but finishes slower is simply a worse genome, and dies.
4. **Its sense of "possible" is inherited from the oracle.** The route graph
   defines which stands are reachable, and the waypoint compass steers the
   whole population along it. Exploits live outside that envelope by
   definition, and nothing ever looks there. Even the genius splash fires
   only on a progress spike (>1.5× previous max), never on a new technique.

The physics, meanwhile, is rich in unused trick potential: stomp bounces
retain 0.85× jump velocity and flyers are stompable (enemies as stepping
stones, or as a lift); springs preserve full horizontal momentum
(sprint-spring leaps that skip stands); a wall jump is worth a full jump's
height and the wall slide is slow enough (0.06) to line up the next one
(sealed-shaft climbs); crumble tiles burn an 8-block fuse that can in
principle be surfed; movers carry riders, so a jump at peak deck velocity
inherits the carry; and coyote time (4 ticks), jump buffering (6) and the
0.45 jump cut reward sub-tile, sub-block precision no human hits reliably.

**Standing policy for this phase: tech, not bugs.** When the AI finds a
physics quirk, the default is to catalogue it as a technique, not patch it.
The version trio already pins champions to their physics, so a discovered
trick stays stable and collectible. Patching is reserved for quirks that
break determinism or make levels trivially void (e.g. leaving the world
bounds), and each patch is a physics version bump with the cost stated.

### WP11. The mastery league: don't stop at solved

- **Change.** When a run strict-solves a level, it enters a second phase
  instead of stopping (a mode, `--mastery` / panel toggle; `--stop-on-win`
  remains for measurement runs): the population of winners keeps evolving
  with the objective flipped to **pure time** (win first, then frames, then
  fewest deaths as tiebreak), the jump penalty removed, and the death
  penalty kept. Selection is among winners only; the solved champion of
  record is never displaced by a league genome that does not also validate
  under the strict protocol.
- **Why.** Optimisation pressure with nothing left to spend it on except
  speed is what produces shortcut-hunting: every frame saved has to come
  from a better line, a braver jump, an earlier take-off.
- **Version impact:** none — league scoring is a run mode with its own
  recorded objective label, like the suite aggregate; the crowning fitness
  (v9/v10) is untouched.
- **Gate.** On three pack levels: median validated win time improves by a
  stated margin (propose ≥10%) over the first-solve champion, and at least
  one league champion per level uses a measurably different route (route-hop
  sequence differs from the first solver's).
- **Effort:** S–M (trainer mode + objective; the reporting exists).

### WP12. TAS polish: a 1-tick optimiser over the champion's run

- **Problem.** The policy decides every 4 ticks (`DECISION_TICKS`). Tricks
  that need 1–3 tick precision — a take-off on the last sub-tile offset, a
  jump cut one tick earlier, a buffered press landing inside the coyote
  window — are inexpressible to the population, however long it evolves.
- **Change.** A post-solve sequence optimiser (`train --tas-polish`): take
  the validated champion's *action sequence* (not its genome), and run a
  small CEM / coordinate search over per-block action substitutions and
  hold-length tweaks at full 60 Hz resolution, scored purely on frames-to-
  win with death = reject. Deterministic seeding, same as everything else;
  the result is stored as a *route artifact* (like a plan route), never as
  a champion genome, and labelled as machine-polished so the distinction
  between "evolved" and "polished" stays honest in the UI.
- **Version impact:** none (route artifacts are not champions).
- **Gate.** On the WP11 levels: polish finds a faster route than the league
  champion on at least one, or proves the league champion locally optimal
  at block granularity on the others (a recorded negative is a result).
  Any polished route must replay bit-identically from its artifact.
- **Effort:** M (the replay machinery exists; the search is small).

### WP13. Reward firsts; preserve freaks (technique archive)

- **Change, in two halves that land together.**
  1. **Extend episode stats** with the technique counters the sim already
     half-tracks: stomps exist; add wall jumps, spring launches, crumble
     tiles burned while stood on, max stomp chain, max airtime, peak
     |velocity|, decks ridden (partly present via deck bonus). Stats are
     reporting, not fitness — no version impact.
  2. **A technique archive beside the QD archive**, keyed by technique
     signature bins (stomps, wall jumps, spring launches, airtime), filled
     like MAP-Elites but never culled by progress: a cell's incumbent is
     the genome that did the *most* of that technique, full stop. Plus a
     **firsts bonus in league/discovery scoring only** (never in the
     crowning fitness): the first genome in a run to achieve a technique
     first (first stomp chain ×3, first sprint-spring launch, first
     crumble surf across ≥3 tiles) gets a one-generation rarity spotlight —
     bred from as a fine parent for a few generations, win or lose.
- **Why.** Real exploit lineages start as useless tricks. The main search
  will always kill them for slowness; something has to keep them alive
  until a later mutation weaponises them. This also builds the raw
  material for a skill library: mastered technique demonstrations,
  extracted from archive incumbents, reusable as WP4 segment seeds on new
  levels — the AI visibly *remembering* a trick is its own wow.
- **Version impact:** none (stats + archive are trainer/reporting).
- **Gate.** Instrumentation first: run the current default on the pack and
  record how often each technique occurs at all (the baseline may be near
  zero for stomps-as-transport — that is the finding, either way). Then:
  technique-archive runs must produce at least one validated champion
  whose route uses a technique the baseline population never produced, on
  at least one battery/pack level.
- **Effort:** M (stats plumbing through `EpisodeStats` consumers is the
  bulk; the archive is a second instance of an existing pattern).

### WP14. Hunt tricks on purpose: oracle-defying search + shortcut hunter

- **Change.**
  1. **Oracle-defying search.** A diagnostic-turned-tool (`train
     --trick-hunt <level>`): beam search over sim snapshots (the WP6
     anytime planner's machinery) with *non-goal* objectives — max height,
     max airtime, max speed, ground-free distance — and a standing target
     list: every stand the reachability oracle marked **unreachable**.
     Any trajectory that reaches one, or beats the oracle's assumed jump
     envelope (height/carry from `physics.rs` derivations), is saved as a
     replayable *discovery artifact* with the trajectory attached.
  2. **Shortcut hunter.** For every validated champion (and every TAS
     route), diff its route-hop sequence against the level's intended
     route graph: stands skipped, hops bypassed, order violations. Report
     a skip percentage and the first skipped stand. Runs automatically at
     the end of a mastery run (WP11) and inside `bench`.
- **Why.** This converts luck into a search: instead of hoping evolution
  stumbles outside the oracle's envelope, aim a search directly at the
  envelope's edge. The shortcut percentage is also the most communicable
  number the game produces ("the AI skipped 40% of your level").
- **Version impact:** none.
- **Gate.** Trick hunt across pack + battery inside the WP6 planner budget
  per level: every discovery artifact replays bit-identically, and every
  claimed envelope violation is re-derived against the physics constants
  before it is recorded (no false "impossible" headlines).
- **Effort:** M (objectives and diffing are new; the search is WP6's).

### WP15. Style championships: constraints breed alien solutions

- **Change.** Alongside the champion of record, per-level *style titles*,
  each a constrained run seeded from the champion population: **Fastest**
  (WP11's league winner), **Fewest jumps**, **No sprint**, **No wall
  jump**, **Stomp-only crossings** (crossings of gaps ≥3 tiles must begin
  with a stomp), **Minimalist** (fewest total inputs). Constraints are
  enforced as action masks / scoring rules recorded on the artifact; a
  title is awarded only to a strict-validated winner under its constraint.
  The level's shelf card grows a gallery: up to six solutions, each a
  replay.
- **Why.** Banning the obvious tool is the most reliable trick-invention
  machine there is — it is how speedrun categories generate tech — and a
  gallery of different solutions is a better show than one champion.
- **Version impact:** none (constraints are run configuration, recorded
  on the artifact like the net shape already is).
- **Gate.** On two pack levels, at least three titles each are won by
  routes whose hop sequences pairwise differ. A title nobody can win is
  retired or re-scoped, in writing.
- **Effort:** M (masks are easy; the gallery UI is the real cost — the
  shelf and replay machinery exist to extend).

### WP16. Show the discovery (presentation — owner decisions)

Half the wow is noticing. Packages:

1. **Discovery detector + naming.** A classifier over champion/league
   trajectories detects technique firsts and envelope violations (from
   WP13/WP14 data) and fires the presentation below. Names are generated
   from a small grammar over (mechanic, effect): *Springboard Skip*,
   *Corpse Bridge* (stomp chain), *Fusesurf*, *Shaft Requiem* — a fixed
   hand-written table, not a language model; names must be stable per
   discovery so players can talk about them.
2. **The moment.** On a discovery during a watched run: slow-motion
   replay of the trajectory segment, the intended route drawn faintly
   underneath the found route, the discovery's name and one line of
   explanation ("crossed the pit on three walkers — the oracle said this
   gap needed the mover"). The existing genius splash is the shell; this
   replaces its progress-only trigger for technique events.
3. **Trick codex.** A per-level (and global) journal page: techniques
   discovered, by whom (which run), replay attached, plus the shortcut
   percentage history. Filling the codex is a collection goal the player
   pursues by building levels that provoke discoveries.
4. **"Break my level" mode.** The player picks a level; the system's
   stated job is to find the skip, the cheese, the unintended route —
   WP14's hunt + WP11's league, presented as a report: intended route,
   found route, skip %, discoveries, replays. This is level-design QA as
   entertainment, and every discovery it makes feeds WP13's archive.
5. **"Can you do THAT?"** From any discovery replay, a playtest challenge:
   the player attempts the trick with the AI's ghost running beside them
   (the ghost machinery already races champions in playtest).

- **Version impact:** none.
- **Gate.** The fun gate, extended: a watcher who has never seen the game
  should be able to say what the AI discovered, unprompted, after one
  mastery run. If they can only say "it got better", the presentation
  failed even if the search succeeded.
- **Effort:** M–L (detector + codex + mode; mostly window work).
- **Owner decisions:** discovery naming tone, whether Break-my-level runs
  automatically after a solve or only on request, and whether TAS-polished
  routes (WP12) may appear in the codex beside evolved ones (labelled).

---

## 8. Sequencing

```
Phase A (WP1–WP3)          no versions move; land any time, each behind its arm
        │
Phase B (WP4 → WP5 → WP6)  WP4 first (no version move) and judged on l11;
        │                  WP5 moves fitness to v10 → ONE pack retrain;
        │                  WP6 any time after WP4 (planner is independent)
Phase C (WP9 early,        no versions move; WP7/WP8 judged at equal episodes;
        WP7, WP8)          WP8 depends on nothing in B but pairs naturally
        │                  with WP4's segment machinery
Phase D (WP10 = v14)       sensors v13 → v14 → ONE pack retrain; last, so the
                           perception retrain is not repeated for B or C
Phase E (WP11–WP16)        no versions move; WP11/WP12 need only a level that
                           solves, so they can start as soon as Phase A's
                           arms land; WP13 pairs with WP7's diversity work;
                           WP14 rides on WP6's anytime planner; WP15 after
                           WP11; WP16 last (it presents what the others find)
```

Retrain windows: exactly two in the whole plan (fitness v10, sensors v14).
If WP5's gate fails, its window disappears and Phase D's is the only one.
Phase E adds no window: league scoring, technique stats, constraints and
polish all live outside the crowning fitness and the sensor vector.

## 9. The adversarial battery (build before Phase B's gate can run)

The pack (l1–l11) does not contain levels that isolate the skills this plan
claims to add. Author one small level per skill, each verified solvable by
oracle + planner, each expected to *fail* under the current default run —
a battery level the current system already solves tests nothing:

| Level | Skill isolated | Current expected failure |
|---|---|---|
| `bat-crumble-bridge` | leave before the fuse ends | no fuse observable; reflex lingers |
| `bat-double-threat` | two converging walkers / walker + flyer | second threat invisible |
| `bat-patrol-wait` | wait a full patrol round trip (>600 frames) | stuck timer + no memory |
| `bat-backtrack` | cross, return, cross again on a different phase | route graph is forward-monotone; waypoint never points back |
| `bat-pit-bait` | deceptive near route over a pit, real route longer | geodesic progress rewards the bait |
| `bat-spring-chain` | chained spring launches with mid-air correction | per-crossing oracle knows it; policy timing does not |
| `bat-deck-phase` | board the second of two movers on the right phase | second mover unphased in observation |

The battery joins the difficulty table as a permanent regression gate: every
future default change reports pack + battery together.

## 10. What this plan deliberately does not do

- **Full NEAT / topology evolution.** No evidence capacity binds: AI_REVIEW's
  h32 arm was *worse* on l9, and the repo's own capacity arm found no gain.
  Shape changes here are deliberate and level-driven (WP2), not evolved.
- **Deep RL (PPO and friends).** Sample-hungry by orders of magnitude against
  a 19 ms generation, hostile to the determinism contract, and aimed at a
  problem — raw search power — that the measurements say this project does
  not have.
- **Bigger single populations.** Measured: pop 800 × 100 costs ~3.8× the
  episodes per solve of pop 200 × 400 (AI_REVIEW §3). Islands (WP7) spend
  the same silicon on diversity instead of duplicates.
- **GPU training.** See §6. The 1660 Super is the renderer's card.

## 11. Success criteria for the plan as a whole

1. **l11-the-elements falls to an evolved, validated champion** inside WP4's
   stated budget. This is the plan's headline and its first gate.
2. **Pack + battery difficulty table (24 seeds):** every level strict-solved
   by the default run configuration — no operator flags — inside its budget.
3. **The editor verdict (WP6) is never wrong in the dangerous direction:** no
   level labelled solvable-by-plan that the planner cannot in fact route,
   across pack + battery + a fuzz set of generated levels.
4. **No performance regression on the baseline machine:** watched runs still
   animate inside the existing frame budget with the thinking dial at its
   default; headless throughput within 20% of today's episodes/s at equal
   configuration, WP7's islands excepted (judged at equal episodes).
5. Every claim above recorded in FINDINGS.md under the §2 protocol — or the
   package that promised it is retired, in writing, in the same place.
6. **The wow criterion (Phase E):** across pack + battery, the system produces
   at least three verified discoveries — a route that skips ≥1 intended hop,
   a trajectory that reaches an oracle-unreachable stand or beats the jump
   envelope, or a technique-first (WP13) that no baseline run produced — each
   replayable bit-identically from its artifact and each surfaced to a
   watcher through WP16's presentation. A discovery nobody is shown does
   not count.
