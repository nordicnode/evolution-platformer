//! Fitness function v1 (§7.4) and the one episode driver everything shares.
//!
//! **Versioned** (§8): any change to a reward term, to the action-repeat cadence,
//! or to the termination rules bumps [`FITNESS_VERSION`]. Champion files record
//! it, and a champion is only loadable when the version matches.
//!
//! Anti-exploit rule (from "Surprising Creativity of Digital Evolution"): reward
//! task progress plus survival, never a proxy. No term rewards jump height,
//! airtime, stomps, or distance travelled backwards — agents farm proxies.
//!
//! v2 adds one **negative** term, [`JUMP_PENALTY`]: jumping costs a little. That is the
//! opposite of a proxy reward — it charges the *appearance* of movement instead of paying
//! for it — and it exists because the v1 landscape made hopping free. With jumping
//! edge-triggered in the sim (physics v2) a held button no longer bunny-hops, and this
//! term is what tells a population that a hop buying no progress is not the same thing as
//! progress. The smallest useful jump earns about eight times its cost, so it can never
//! teach an agent not to jump when jumping is the answer.
//!
//! ## v3: the park trap, and the end of the survival bonus
//!
//! v3 **deletes the per-frame survival term** and shrinks [`DEATH_PENALTY`] from seven and
//! a half tiles of progress to one and a quarter. Both were measured, on a level built to
//! the shape of the one that produced the report: a flat floor with one-tile spikes and one
//! safe tile between them, which the population could not learn at all.
//!
//! The best of 4 800 evolved agents walked to x = 7.4 — the last tile before the first spike
//! — made a single jump and then **stood there for 1 760 frames**. That genome scored 169,
//! and 88 of it was survival: standing still was, by construction, worth as much as all the
//! ground it had actually covered. Every genome that *tried* the jump died at the spike for
//! about -40, so the hill pointed at the wall.
//!
//! The arithmetic is worth stating, because it decides the constants. Crossing a hazard was
//! worth ~2 tiles of frontier (32) and a death cost 120, so attempting paid only above a
//! **79%** success rate — and with the parker banking 88 survival on top, a genome had to
//! clear *six or seven* hazards in a row before trying the first one was worth anything at
//! all. No population learns through that. With the survival term gone and a death priced at
//! 1.25 tiles, the same calculation turns over at about **one hazard in four**, which is a
//! gradient a GA can climb.
//!
//! Neither change is a proxy reward and neither pays for dying: progress is still measured as
//! the rightmost ground *ever* reached (`EpisodeStats::best_x`), so a death can only forfeit
//! the progress it failed to make — it can never bank any. What v3 removes is a term that
//! paid for *not playing*. See `clearing_a_hazard_and_dying_beats_parking_in_front_of_it`,
//! which is this whole argument as an assertion.
//!
//! ## v4: checkpoints, the half of a failed leap the frontier cannot keep
//!
//! v4 adds [`CHECKPOINT_BONUS`]: a safe landing zone the body **stood on** past a hazard or
//! a hole pays once. It exists for the case finding 15 named — the cul-de-sac, where a pit
//! gives the population no feedback until a jump succeeds — and it is deliberately the
//! *stricter* half of the frontier rather than a second frontier.
//!
//! The distinction is the whole reason the counting lives in the sim. `best_x` is credited
//! for any x the body reaches, **including the airspace over a pit it then falls into**, so
//! a failed leap already keeps the distance it covered; what it cannot keep is the landing.
//! So a genome that sails over a hazard and dies scores the frontier and nothing else, while
//! the same leap that arrives scores both. A term that could be collected by flying over a
//! hazard would be a proxy for distance covered under a new name — this one cannot be, and
//! the test that pins it (`a_checkpoint_is_credited_on_arrival_and_not_for_flying_over_the_zone`)
//! asserts the count is still zero while the body is airborne *inside* the zone's columns.
//!
//! **Measured** on the level shape the owner's report produced (one-tile spikes with one
//! safe tile between them — the tightest hazard floor in the repo, and the one that a whole
//! frontier-only fitness never solved at any population up to 1000):
//!
//! | budget (three seeds) | v3, frontier only | v4, with checkpoints |
//! |---|---|---|
//! | pop 200 × 120 gens | unsolved — `best_x` 11.8 / 13.3 / 11.8 | **solved at generation 107** (seed 2); 12.5 / 13.4 on the others |
//! | pop 500 × 400 gens | unsolved — 11.9 / 11.9 / 11.9 | unsolved, but 12.6 / 18.9 / **35.4** |
//! | pop 1000 × 800 gens | solved once, at generation 110 of 800 | solved at generation 313 (seed 2); 12.8 / 14.6 on the others |
//!
//! Two things to read out of that table. The headline is the middle row: v3's progress
//! plateau was *one* hazard crossed (it stalled at the first spike for 400 generations at
//! every seed), and v4's is **thirteen of fifteen** — a population that used to have nothing
//! to climb on now walks the whole floor and dies at the last pad. The other is the first
//! row: the level that needed a population of 1000 to fall was solved at 200, which is the
//! term buying a solve for a fifth of the search.
//!
//! ## v5: the objective is the goal, not the right-hand edge of the screen
//!
//! v5 changes no term. It changes the *sign* of the thing every term is measured against:
//! `progress_pixels` was "new rightmost x", which silently defined this game as a
//! left-to-right side-scroller. Two consequences, both of which a level designer hits the
//! first time they build a tower:
//!
//! * **Walking beat climbing.** On a level whose flag is up, moving right along the floor
//!   paid and moving up paid nothing, so the best genome in a 200-strong population walked
//!   to the far wall of a 10-wide tower for the whole episode and never jumped once
//!   (measured: `best_x` 9.4 of 10, one jump, timed out).
//! * **`Stuck` killed climbers.** The anti-camper rule fires after 600 frames without a new
//!   rightmost x, so a body making steady upward progress was terminated as a camper. There
//!   was a third block behind those two, in the frame cap: `width × FRAMES_PER_TILE` gave a
//!   10x22 tower 400 frames to climb 18 rows, fewer frames than the 600-frame timer it was
//!   racing.
//!
//! Fixed together, because fixing one alone leaves the level unplayable in a different way:
//! progress is now the **distance closed on the flag** (from the spawn, so a level never
//! starts in credit), the stuck timer measures closing that distance, the frame cap is the
//! level's *longer* side, and the observation's one horizontal goal sign became a two-axis
//! `goal_dx, goal_dy` vector (sensors v3) so the flag is visible up as well as right.
//!
//! Measured on the vertical level those three bugs were found on
//! (`levels/scratch-tower.json`: 10×22, a zig-zag of shelves, flag eighteen rows above the
//! spawn), pop 300, three seeds: **solved on two of them, at generations 275 and 327**, with
//! the third stalling after 400 generations — still the walking genome that stalls, at
//! `best_x` 9.4 with two jumps and one checkpoint. That last line is the useful one: the old
//! objective's optimum is now the *worst* place on the level, which is what this fix should
//! look like from inside the population.
//!
//! Read that number as *"this level is solvable"*, not as a difficulty. It is a lottery,
//! and the generations move with the sensor layout and the seed: the same three seeds did
//! not solve it at all before the gap probe was split into `gap_right` + `gap_left` (sensors
//! v4, REVIEW.md finding 1.9), while its *mirror* solved on two of three in that budget at
//! the same time. An earlier measurement of the same level at the same budget gave different
//! generations again. The claim this section is really making is the one that survives all of
//! them: no genome parks on the floor any more, because the floor is no longer worth
//! anything.
//!
//! On a rightward level this is the same signal it always was — the goal is far to the right,
//! so closing the distance is moving right — which is why the shipped pack's difficulty came
//! out where it was. It is also the reason the change is a version bump rather than a
//! refactor: on any level whose goal is not due right, every score in the file is a different
//! number now.
//!
//! Honest limits, because a table like that invites over-reading. The easier hazard floor
//! (two safe tiles between spikes) is a *wash*: v3 solved it in 14–69 generations across
//! six budgets, v4 in 18–50. It was already learnable, and a term that helps where the
//! gradient is missing has little to add where it is not. And the solves are per-seed: the
//! tight floor remains a lottery at these budgets either way, just a much better-priced one.
//! What the term did *not* do is weaken anything: `l1-flat-run` and the other hazard-free
//! levels have no checkpoint to collect (`Level::checkpoint_zone` returns `None` everywhere
//! on flat ground, which is asserted), so their landscape is untouched, and the shipped
//! difficulty ladder passes unchanged.
//!
//! ## v8: a win pays for the time it did not spend
//!
//! Every version above is about *getting* to the flag. v8 is the first that cares how long it
//! took, and it adds exactly one term: [`SPEED_BONUS`], paid **only on a win**, scaled by the
//! fraction of the level's own frame budget the episode left unspent.
//!
//! The budget is the denominator, and it is not a new number: `sim::frame_budget` is
//! `max(width, height) × FRAMES_PER_TILE`, the same cap the sim already stops episodes at, so
//! "finished quickly" has a definition that every level already had, needs no per-level par to
//! hand-tune, and cannot drift away from the rule that ends an episode. A very short level and a
//! very long one are both measured against what their own terrain allows.
//!
//! Three properties are the reason the term is shaped this way rather than as a multiplier on
//! the goal:
//!
//! * **It cannot be farmed, because it pays only on a win.** A bonus that scaled *progress* by
//!   speed would make a fast death the best move on the level — the same class of mistake as the
//!   park trap, pointing the other way. A winning episode is the only one that has a time to be
//!   praised for.
//!
//! * **It is a tie-break between winners, not a second objective.** On a level the population can
//!   win, most of a generation arrives at the flag, where progress is saturated: what is left to
//!   rank them by is the hopping they did and how long they took. Before v8 those two were the
//!   only signal a winner had over another winner, and `JUMP_PENALTY` alone is a weak reason to
//!   find a *shorter route* — a genome that hops twelve times and arrives in half the budget beat
//!   one that walks straight and hopped three times, for the same score. Now it does not.
//!
//! * **It is bounded and small against the goal.** `SPEED_BONUS` is worth about twelve tiles of
//!   progress against `GOAL_BONUS`'s ten thousand, so a hundred maximally fast wins still could
//!   not buy what one flag does: it reorders winners, it never makes losing pay.
//!
//! Speed is also, as of the same change, a *behaviour* the archive keeps: `ga::MapElitesArchive`
//! keys its cells on the finish band as well as on progress and checkpoints, so the fastest
//! route found in a niche survives as its incumbent instead of being bred out by a marginally
//! slower genome the moment its lineage stops improving. See `ga::ARCHIVE_TIME_BINS`.

use sim::hash::StateHasher;
use sim::physics::AgentAction;
use sim::{EpisodeStats, Level, Outcome, Sim};

use crate::genome::{ActScratch, Genome};

/// Fitness layout version. Bump on *any* change below (§8).
///
/// v2: the per-jump term below. v3: the survival term removed and the death penalty
/// resized. v4: the checkpoint term below. v5: no term changed — what changed is what
/// `progress_pixels` *is*, from rightmost x to distance-closed-on-the-flag, which re-scores
/// every level in the pack whose goal is not due right. See the module docs, which carry
/// the measurement that set all of them.
/// v6: directional checkpoint arrivals (`Level::checkpoint_zone_from`), so a leftward crossing
/// credits the far landing zone symmetrically with a rightward one.
/// v7: geodesic route-hop bonus (`ROUTE_HOP_BONUS`): each canonical crossing closed toward the
/// flag along the level's move graph (`EpisodeStats::route_hops_closed`) pays a monotone reward,
/// so a zig-zag tower whose route turns away from the flag still has a strictly ascending hill.
/// v8: the speed term below ([`SPEED_BONUS`]) — a win pays for the budget it did not spend —
/// and the archive gained a finish-time axis, which changes what the search *keeps* without
/// changing what it scores.
pub const FITNESS_VERSION: u32 = 9;

/// Physics ticks between network queries (§7.3): 60 Hz sim → a decision every ~67 ms; the
/// chosen action repeats in between. Exposed as a tuning knob; the plan's range is 3–6.
///
/// Read from [`sim::DECISION_TICKS`] rather than declared here, because the cadence is a
/// property of the *agent's control* and not of the reward: the physics derives the
/// shortest jump from it (`Physics::min_jump_hold` is one block), and the observation is
/// sampled on it. Two numbers for one fact is how a trainer ends up scoring a body whose
/// jump ladder is built for a different cadence — and `--action-repeat` can still override
/// it for an experiment, which is why `EvalConfig` carries its own copy.
pub const ACTION_REPEAT: u32 = sim::DECISION_TICKS;

/// Fitness per pixel of ground closed on the flag (§7.4.1).
///
/// "Pixel of ground covered" until 0.5.0, when the measurement it multiplies stopped being
/// rightmost x — see `EpisodeStats::progress_pixels`. The scale is unchanged (16 pixels to
/// the tile, and a straight rightward level produces the same number it always did), so
/// every constant below that was sized against "one tile" still means one tile.
pub const PROGRESS_PER_PIXEL: f32 = 1.0;
/// Fitness on dying (§7.4.3, resized in v3).
///
/// **One and a quarter tiles of progress** (`PIXELS_PER_TILE = 16`), and the size is derived
/// rather than tuned: it has to be smaller than the smallest hazard *crossing*, or a
/// population is better off standing in front of a hazard than trying it. The smallest
/// crossing is a one-tile spike with a landing past it, worth two tiles of new frontier (32),
/// so a death at 20 leaves the attempt profitable well before half the jumps land.
///
/// It still has to be a cost: dying gains nothing (`best_x` only ever rises, so no jump can
/// bank progress by killing the runner), but a free death would make a suicidal genome exactly
/// as good as a careful one that reached the same frontier. `JUMP_PENALTY × 15 ≈ DEATH_PENALTY`
/// on a hop-heavy level, which is the right order — a death is worth about what a dozen
/// wasted hops are worth in information about the genome.
///
/// v1 and v2 priced this at 120 (seven and a half tiles), which was the park trap's other
/// half: see the module docs.
pub const DEATH_PENALTY: f32 = -20.0;
/// Fitness on touching the flag (§7.4.4). Must dominate any progress-only local
/// optimum, which is why it is five figures while a full-width level's progress
/// score is four.
pub const GOAL_BONUS: f32 = 10_000.0;
/// Fitness per jump taken (§7.4.5, new in v2).
///
/// Sized against the progress either side of it: the smallest useful jump — clearing a
/// one-tile gap — earns `PIXELS_PER_TILE = 16.0`, eight times the charge, so it cannot
/// discourage a jump that buys progress. What it does do is break the tie on flat ground,
/// which
/// is where the old landscape had nothing to say at all: `run` and `run_jump` reached the
/// flag in the same number of frames for the same progress, so evolution had no reason to
/// prefer either, and the population's champions jumped 2.58 times per 100 frames — the
/// mechanical rate of a held button.
///
/// v1 and v2 sized this as "exactly what the 40 frames of its own airtime would have
/// earned", because those versions paid for being alive; v3 deleted that term, and what is
/// left to price a hop against is the progress it might have bought rather than the
/// survival it consumed.
pub const JUMP_PENALTY: f32 = -2.0;

/// Fitness for each safe landing zone the body reaches **past a hazard** (new in v4).
///
/// One tile of progress (`PIXELS_PER_TILE = 16`), and the size is derived rather than
/// tuned. The job is to pay for *arriving* on the far side of something, so it has to be
/// legible next to the crossing that earned it — the smallest hazard crossing is worth
/// `2 × PIXELS_PER_TILE = 32` of frontier — while staying well under the frontier's own
/// value so it can never become a thing to farm in place of progress. A hazard field
/// thirteen pads long carries 208 points of it against a 10 000-point goal, which is a
/// nudge to the gradient and not a second objective.
///
/// It is **not** a proxy reward, and the distinction matters more here than anywhere else
/// in this file, because "reached a safe landing zone" sounds exactly like a proxy for
/// distance covered. It is not one, for a reason that makes it *stricter* than the
/// frontier rather than looser: the sim counts a zone only while the body is standing on
/// it, and only once. The frontier is credited for anywhere the body reaches — including
/// the airspace over a pit it then falls into — so a failed leap already keeps the ground
/// it covered. This term is the half that a failed leap cannot keep. It rewards the same
/// progress *and then requires the body to still be alive and on the ground when the dust
/// settles*, which is precisely what a pit does not otherwise give the population any
/// feedback about (finding 15's cul-de-sac, and FINDINGS.md finding 49).
///
/// It cannot be collected at spawn: the zone touching the level's left edge is never a
/// checkpoint, so a level with no hazards pays nothing and its landscape is untouched.
pub const CHECKPOINT_BONUS: f32 = 16.0;

/// Fitness paid for finishing **inside the level's own frame budget** (new in v8).
///
/// The term is `SPEED_BONUS × unspent / budget`, where `unspent` is `budget_frames - frames`
/// from the episode's own stats, so an episode that spent its whole budget earns nothing and a
/// maximally fast one earns the lot. The budget is `sim::frame_budget` — `max(width, height) ×
/// FRAMES_PER_TILE`, the cap the sim already stops episodes at — which is what makes "quickly"
/// a fact about the level rather than a number someone typed in per level.
///
/// **Paid only on a win.** That is the load-bearing half of the design and not a detail: the
/// term's whole job is to rank the winners of a generation, which is where progress has
/// saturated and the only signals left are `JUMP_PENALTY` and the clock. Paying it for any
/// finished episode would make dying fast a strategy — `Outcome::Stuck` and `Outcome::Died` also
/// stop the clock — and that is the park trap with the sign flipped: a race to be killed. A
/// losing episode can never claim it.
///
/// **Twelve tiles of progress**, and the size is derived rather than tuned: one pixel is one
/// point ([`PROGRESS_PER_PIXEL`]) and a tile is sixteen, so 200 is about a fifth of a
/// sixty-tile level's whole frontier. Against `GOAL_BONUS` (10 000) that is 2%, which is the
/// point — the term decides *which* winner is the winner and can never make losing look like
/// winning, since even a hundred clean runs bank less than one flag. Against the other terms a
/// winner's score is made of it is decisive: a champion of a full-width level pays roughly
/// sixty points of `JUMP_PENALTY` over its episode and collects a handful of checkpoints, so a
/// few hundred points of speed is the only term large enough to reshape the *route* rather
/// than the style.
///
/// The honest risk, stated because the number is small enough to be missed: on a level the
/// population wins in its first generation, this changes nothing about *whether* it learns
/// (there is nothing left to learn) and everything about which genome is kept as the champion —
/// the champion being the top-scoring genome, which is what a `Save champion` writes and what
/// the panel calls the best agent so far. It is *not* what the window replays: the replay and
/// the playtest's ghost both follow the quickest **win** (`FastWin`), because those are two
/// agents whenever a slower route collects enough checkpoints and route hops to outscore a
/// quicker one — roughly 50 points of clock on a minute-long level against 32 a hop — and a view
/// that races a time has to show the agent that set it. On a hard level this term is worth
/// nothing until the flag is reached at all.
pub const SPEED_BONUS: f32 = 200.0;

/// Fitness for each canonical route crossing closed toward the flag (`EpisodeStats::route_hops_closed`,
/// new in v7).
///
/// Two tiles of progress (`2.0 × PIXELS_PER_TILE = 32.0`), paid only when grounded on a stand that
/// has strictly fewer crossings left to the flag than any stand reached earlier in the episode.
/// On a zig-zag tower where straight-line Euclidean distance reverses up every switchback leg,
/// `progress_pixels` plateaus across the leg while `route_hops_closed` rises by 1 on every platform,
/// giving evolution a true geodesic gradient all the way to the top.
pub const ROUTE_HOP_BONUS: f32 = 32.0;

/// Fitness for each moving-platform deck the body has **stood on** (new in v9).
///
/// One tile of progress, the same size as a hazard checkpoint and for the same reason: it is
/// an arrival, not a proxy, and it is one-shot per deck. On a level whose crossing is a chain
/// of shuttles, the frontier and the route-hop ladder both only move once the crossing has
/// already succeeded; this is the term that makes "get onto the first deck" a visible step.
/// It cannot be farmed — the body has to be standing on the deck, and a deck pays once — and
/// a level with no mover pays nothing, so no earlier landscape moves.
pub const DECK_BONUS: f32 = 16.0;

/// Knobs the evaluation depends on. Part of the champion metadata, because
/// changing the cadence changes the trajectory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalConfig {
    pub action_repeat: u32,
}

impl Default for EvalConfig {
    fn default() -> Self {
        EvalConfig {
            action_repeat: ACTION_REPEAT,
        }
    }
}

/// Score one finished or in-progress episode.
///
/// Four terms, and the absence of a fifth is the point of v3: **nothing here pays per
/// frame**. Time spent standing still is worth nothing, so the only way up the hill is
/// ground covered — and, since v4, ground *arrived at*. Every term is one-shot or
/// monotone: progress is the closest the body has ever been to the flag (in either
/// direction, since v5), a checkpoint zone pays once however often it is revisited, and no
/// term can be collected by repeating anything.
pub fn score(stats: &EpisodeStats, outcome: Outcome) -> f32 {
    let mut f = stats.progress_pixels * PROGRESS_PER_PIXEL;
    f += stats.jumps as f32 * JUMP_PENALTY;
    f += stats.checkpoints as f32 * CHECKPOINT_BONUS;
    f += stats.route_hops_closed as f32 * ROUTE_HOP_BONUS;
    f += stats.decks as f32 * DECK_BONUS;
    if outcome == Outcome::Died {
        f += DEATH_PENALTY;
    }
    if outcome == Outcome::Won {
        f += GOAL_BONUS;
        // The clock, and only for an agent that actually got there: see [`SPEED_BONUS`], where
        // the reason this is inside the `Won` arm — rather than paid to anything that stopped —
        // is the whole design. An episode whose budget is unknown (a stats value built by hand
        // rather than by the sim) has nothing to measure against and scores the same as it did
        // before this term existed, which keeps the arithmetic finite in every caller.
        if stats.budget_frames > 0 {
            let unspent = stats.budget_frames.saturating_sub(stats.frames) as f32;
            f += SPEED_BONUS * (unspent / stats.budget_frames as f32);
        }
    }
    f
}

/// Everything one episode produced. Index-aligned with the population by the
/// evaluator, so `fitness[i]` always belongs to `genomes[i]` (§5.6).
#[derive(Clone, Debug, PartialEq)]
pub struct EpisodeResult {
    pub genome_id: u32,
    /// Fitness of the **representative episode** (trial 0). This is the number a
    /// champion records and the number `replay` must reproduce exactly, which is
    /// why it is always trial 0 rather than whichever trial happened to score best.
    ///
    /// On a **suite** ([`crate::ga::evaluate_suite`]) the representative episode is trial 0 of
    /// the suite's **first** level, and that is deliberate: a champion is a claim about one
    /// level, so the file has to record an episode somebody can replay — the suite score is a
    /// selection metric and lives in [`EpisodeResult::mean_fitness`].
    pub fitness: f32,
    /// Mean fitness over all trials; equal to `fitness` when `trials == 1`.
    ///
    /// Selection ranks on this. A single episode is noisy evidence about a genome,
    /// but it is also the only number a third party can reproduce from one seed,
    /// so the two are kept apart instead of being conflated: without this split, a
    /// champion trained with `--trials 4` would advertise an average and then fail
    /// its own replay check.
    ///
    /// On a **suite** it is not a mean of trials at all but the *aggregate* across levels —
    /// the mean of the level this genome is worst at (see [`crate::ga::evaluate_suite`] for the
    /// arithmetic, and for why it is the minimum rather than a sum). Everything that records a
    /// number about a genome reads [`EpisodeResult::fitness`]; everything that *ranks* one
    /// reads this.
    pub mean_fitness: f32,
    /// Fitness of the **lowest-scoring trial** this genome was judged on; equal to
    /// [`EpisodeResult::mean_fitness`] when it was judged on one, and equal to
    /// [`EpisodeResult::fitness`] on a single episode.
    ///
    /// The pessimistic half of "what did this genome do". A mean over three patrol phases says a
    /// genome is good on average; this says it was never worse than *this*, and on the levels
    /// where the two come apart it is the number that matters: a genome that solved two phases
    /// and died at the walker on the third has a good mean and a bad floor, which is exactly the
    /// failure `REVIEW.md`'s finding 14 records — an elite re-rolled out of the run by a phase it
    /// never saw. Nothing ranks on it unless the operator asks
    /// ([`crate::ga::GaConfig::rank_pessimism`]); like the mean, it is a *reading*, and the
    /// recorded number remains [`EpisodeResult::fitness`].
    ///
    /// Set by [`crate::ga::suite_score`], the one function that ever runs more than one episode
    /// for a genome; a single episode is its own floor, which is what makes the field exactly
    /// `mean_fitness` on every deterministic level and keeps a memoryless run's ranking
    /// bit-identical whatever the pessimism is set to.
    pub worst_trial: f32,
    pub outcome: Outcome,
    pub frames: u32,
    pub progress_pixels: f32,
    pub best_x: f32,
    pub stomps: u32,
    /// Jumps taken, so a watcher can tell "solved it" from "bounced all the way there".
    pub jumps: u32,
    /// Safe landing zones reached past a hazard, so a watcher can tell how far a genome
    /// got that the frontier alone does not say: a body that sailed over a pit and fell in
    /// has frontier but no checkpoints.
    pub checkpoints: u32,
    /// Route crossings closed toward the flag while grounded (fitness v7).
    pub route_hops_closed: u32,
    /// Deepest route stand index reached while grounded in this episode.
    pub best_stand: Option<usize>,
    /// Levels of a **suite** this genome won, when it was judged on more than one level
    /// ([`crate::ga::evaluate_suite`]). `0` on an ordinary single-level run.
    ///
    /// A count of *wins*, not of levels evaluated: a genome that reached ninety percent of
    /// every level in a three-level suite has zero, because nothing on a suite is solved by
    /// being close. It is the number that makes "the suite fell" a statement a reader can
    /// check — the run's own solve signal is every level won, not one.
    pub suite_wins: u32,
    /// **Every** episode this genome was judged on reached the flag, over every level and every
    /// trial — the conservative reading of "solved", and the one a run's success signal uses.
    ///
    /// [`EpisodeResult::won`] answers a narrower question: did the *representative* episode (trial
    /// 0 of the first level) win. That is the right question for a champion file, because a
    /// champion is a claim about one replayable episode — but it is the wrong question for "has
    /// this run solved the level", and on a walker level the difference is not academic. A patrol's
    /// phase is drawn per episode, so a genome can win the one phase it was scored on and be a
    /// coin flip on the next; `won` calls that solved and this field does not.
    ///
    /// A single-episode evaluation is its own whole, so this equals [`EpisodeResult::won`] wherever
    /// a level is deterministic. Only [`crate::ga::suite_score`] can set it to anything else, and it
    /// folds every recorded episode's outcome — a suite's `won` is level 0, trial 0, while this is
    /// every level and every trial.
    ///
    /// One caveat, and it is a property of racing rather than a bug: a genome culled before its
    /// second trial was never judged on one, so its flag describes the episodes that exist. Racing
    /// culls on the score, and a trial-0 winner scores the goal bonus, so a winner is never the
    /// genome a cull removes.
    pub all_trials_won: bool,
    pub death_pos: Option<(f32, f32)>,
    /// What ended a dying episode, when the stepper recorded one. Diagnostics only: it is not
    /// scored and not part of the trajectory hash.
    pub death_cause: Option<sim::DeathCause>,
    pub seed: u64,
    /// Hash of the decision trace and final state. Champion round-trip tests
    /// compare this, so a replay that diverges anywhere is caught.
    pub trajectory_hash: u64,
}

impl EpisodeResult {
    /// Did the **representative** episode reach the flag? The champion's own claim: trial 0 of the
    /// first level, which is the episode `replay` reproduces.
    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
    }

    /// Did **every** episode this genome was judged on reach the flag? The run's solve signal —
    /// see [`EpisodeResult::all_trials_won`] for why the two questions are different.
    pub fn solved(&self) -> bool {
        self.all_trials_won
    }
}

/// Step-by-step episode driver.
///
/// [`run_episode`] is this plus a loop, deliberately, so the decision cadence has
/// exactly **one** implementation. Watch mode drives this directly to animate a
/// genome; if it re-implemented the cadence, a watched champion could disagree
/// with the fitness the trainer recorded for it.
///
/// It **owns** its genome rather than borrowing it. A viewer draws a whole
/// generation at once, so it holds a pool of runners beside the genomes they came
/// from; if a runner borrowed its genome, that pool would be a self-referential
/// struct, which is the one shape the owner of both cannot express.
#[derive(Debug)]
pub struct EpisodeRunner {
    genome: Genome,
    sim: Sim,
    scratch: ActScratch,
    action: AgentAction,
    repeat: u32,
    seed: u64,
    trace: StateHasher,
    iterations: u32,
    guard: u32,
}

impl EpisodeRunner {
    pub fn new(level: &Level, genome: Genome, cfg: &EvalConfig, seed: u64) -> Self {
        let repeat = cfg.action_repeat.max(1);
        let sim = Sim::new(level, seed);
        // `max_frames` is the sim's own cap; the extra slack is a belt-and-braces
        // guard against a future change that forgets to terminate.
        let guard = sim.max_frames() + repeat + 8;
        EpisodeRunner {
            genome,
            sim,
            scratch: ActScratch::new(),
            action: AgentAction::Walk,
            repeat,
            seed,
            trace: StateHasher::new(),
            iterations: 0,
            guard,
        }
    }

    /// Live view of the episode. Read-only, so a renderer cannot disturb it.
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn genome(&self) -> &Genome {
        &self.genome
    }

    /// One physics tick. Queries the network every `action_repeat` ticks.
    pub fn step(&mut self) {
        if self.is_finished() {
            return;
        }
        let frames = self.sim.stats().frames;
        if frames % self.repeat == 0 {
            self.action = self.genome.act(&self.sim.observation(), &mut self.scratch);
            self.trace.u32(frames);
            self.trace.u8(self.action.index() as u8);
        }

        self.sim.step(self.action);
        self.iterations += 1;
    }

    /// The episode ended, or the safety guard fired.
    pub fn is_finished(&self) -> bool {
        self.sim.outcome().is_finished() || self.iterations >= self.guard
    }

    /// The action currently being held.
    pub fn action(&self) -> AgentAction {
        self.action
    }

    /// Finish the episode and score it.
    pub fn result(mut self) -> EpisodeResult {
        self.trace.u64(self.sim.state_hash());
        let outcome = self.sim.outcome();
        let stats = self.sim.stats();
        let fitness = score(stats, outcome);
        EpisodeResult {
            genome_id: self.genome.id,
            fitness,
            mean_fitness: fitness,
            // One episode was played, so it is its own floor. `suite_score` lowers this as it
            // runs the rest of a genome's trials, and it is the only thing that can.
            worst_trial: fitness,
            outcome,
            frames: stats.frames,
            progress_pixels: stats.progress_pixels,
            best_x: stats.best_x,
            stomps: stats.stomps,
            jumps: stats.jumps,
            checkpoints: stats.checkpoints,
            route_hops_closed: stats.route_hops_closed,
            best_stand: stats.best_stand,
            // One level was played, so no suite was solved: `evaluate_suite` is the only
            // thing that raises this, and it raises it after this episode is finished.
            suite_wins: 0,
            // One episode was played, so it is the whole of what this genome is judged on, and
            // its own outcome is the conservative answer. `SuiteTally::result` can only lower it.
            all_trials_won: outcome == Outcome::Won,
            death_pos: stats.death_pos,
            death_cause: self.sim.death_cause(),
            seed: self.seed,
            trajectory_hash: self.trace.finish(),
        }
    }
}

/// Run one episode: the **single** implementation used by training, by champion
/// replay, and by the round-trip test. If these ever diverged, a champion would
/// stop reproducing its advertised fitness.
///
/// The loop is deliberately dumb: decide every `action_repeat` ticks, step once
/// per tick, no accumulator, no wall-clock.
pub fn run_episode(
    level: &Level,
    genome: &Genome,
    cfg: &EvalConfig,
    seed: u64,
) -> EpisodeResult {
    let mut runner = EpisodeRunner::new(level, genome.clone(), cfg, seed);
    while !runner.is_finished() {
        runner.step();
    }
    runner.result()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{default_shape, Genome};
    use crate::fitness::EpisodeRunner;
    use sim::physics::AgentAction;
    use sim::rng::rng_from_seed;
    use sim::EpisodeStats;

    /// A synthetic episode on a sixty-tile level: 60 × `FRAMES_PER_TILE` = 2400 frames of
    /// budget, which is the shape every shipped level has and what [`SPEED_BONUS`] is measured
    /// against.
    fn budget() -> u32 {
        60 * sim::FRAMES_PER_TILE
    }

    fn stats(progress_pixels: f32, frames: u32) -> EpisodeStats {
        EpisodeStats {
            frames,
            budget_frames: budget(),
            progress_pixels,
            // Unused by `score` (which reads totals, never rates), so a synthetic episode has
            // no need of a trajectory to have produced them.
            closest_approach: 0.0,
            goal_distance_pixels: 0.0,
            best_x: 0.0,
            no_progress_frames: 0,
            stomps: 0,
            jumps: 0,
            checkpoints: 0,
            route_hops_closed: 0,
            best_stand: None,
            decks: 0,
            death_pos: None,
        }
    }

    fn stats_with_jumps(jumps: u32, progress_pixels: f32, frames: u32) -> EpisodeStats {
        EpisodeStats {
            jumps,
            ..stats(progress_pixels, frames)
        }
    }

    fn stats_with_checkpoints(
        checkpoints: u32,
        progress_pixels: f32,
        frames: u32,
    ) -> EpisodeStats {
        EpisodeStats {
            checkpoints,
            ..stats(progress_pixels, frames)
        }
    }

    #[test]
    fn the_step_driver_agrees_with_the_whole_episode_driver() {
        // Watch mode animates through `EpisodeRunner`; training scores through
        // `run_episode`. If the two ever disagreed about the decision cadence, a
        // watched champion would contradict its own recorded fitness -- which is
        // the one thing watch mode exists to make trustworthy.
        let level = level();
        for seed in [1u64, 7, 4242] {
            let mut rng = rng_from_seed(seed);
            let g = Genome::random(seed as u32, default_shape(), &mut rng);
            let cfg = EvalConfig::default();

            let whole = run_episode(&level, &g, &cfg, seed);

            let mut runner = EpisodeRunner::new(&level, g.clone(), &cfg, seed);
            let mut ticks = 0u32;
            while !runner.is_finished() {
                runner.step();
                ticks += 1;
                assert!(ticks < 10_000, "the step driver failed to terminate");
            }
            let stepped = runner.result();

            assert_eq!(stepped, whole, "seed {seed}");
        }
    }

    #[test]
    fn the_step_driver_matches_for_every_action_repeat() {
        let level = level();
        let mut rng = rng_from_seed(3);
        let g = Genome::random(1, default_shape(), &mut rng);
        for repeat in [1u32, 3, 4, 6, 16] {
            let cfg = EvalConfig { action_repeat: repeat };
            let whole = run_episode(&level, &g, &cfg, 5);
            let mut runner = EpisodeRunner::new(&level, g.clone(), &cfg, 5);
            while !runner.is_finished() {
                runner.step();
            }
            assert_eq!(runner.result(), whole, "action_repeat {repeat}");
        }
    }

    #[test]
    fn a_single_trial_reports_the_same_number_twice() {
        let g = Genome::zeros(0, default_shape());
        let r = run_episode(&level(), &g, &EvalConfig::default(), 1);
        assert_eq!(
            r.fitness, r.mean_fitness,
            "with one trial there is nothing to average"
        );
    }

    #[test]
    fn a_goal_beats_any_progress_only_local_optimum() {
        let level_width = 60.0;
        let perfect_progress = level_width * sim::PIXELS_PER_TILE;
        let best_without_goal = score(&stats(perfect_progress, 2400), Outcome::TimedOut);
        let worst_win = score(&stats(0.0, 1), Outcome::Won);
        assert!(
            worst_win > best_without_goal,
            "touching the flag ({worst_win}) must outweigh a full-width run ({best_without_goal})"
        );
    }

    /// **A deck is worth a rung, once.**
    ///
    /// The v9 term is an arrival, not a distance: the episode counts distinct decks stood on, so
    /// two landings on the same deck pay one bonus and a second deck pays another. It is small
    /// and one-shot because it is a shaping term, not a win: the flag is still the objective, and
    /// a level with no mover must score exactly as it did before v9.
    #[test]
    fn a_deck_pays_once_and_only_when_it_was_stood_on() {
        let none = stats(0.0, 100);
        let one = EpisodeStats {
            decks: 1,
            ..stats(0.0, 100)
        };
        let two = EpisodeStats {
            decks: 2,
            ..stats(0.0, 100)
        };
        let base = score(&none, Outcome::Running);
        assert_eq!(
            score(&one, Outcome::Running) - base,
            DECK_BONUS,
            "the first deck pays its one bonus"
        );
        assert_eq!(
            score(&two, Outcome::Running) - base,
            2.0 * DECK_BONUS,
            "each further deck is its own arrival"
        );
    }

    /// **The park trap, as an assertion** — the measured failure v3 exists for.
    ///
    /// A genome that stands in front of a hazard until the clock runs out must score *below*
    /// one that clears the hazard and dies on the far side of it. The two episode shapes are
    /// the ones actually observed on a one-tile-safe-gap spike field: the best of 4 800
    /// evolved agents parked at x = 7.4 (5.4 tiles of frontier) for 1 760 frames, while every
    /// genome that took the jump died at the first spike.
    ///
    /// Under v2 this test fails, and that is not a detail — it is *why* the population refused
    /// to jump. Parking banked 88 survival on top of its 86 progress, and the attempt scored
    /// about -40, so the gradient ran away from the hazard and a genome had to clear six or
    /// seven of them before trying the first one paid.
    #[test]
    fn clearing_a_hazard_and_dying_beats_parking_in_front_of_it() {
        // The frontier the measured parker reached, then nothing for 1 760 frames.
        let parked = score(&stats(5.4 * sim::PIXELS_PER_TILE, 1760), Outcome::Stuck);
        // Two tiles further: one hop over a one-tile spike, and then the next one kills it.
        let attempted = score(&stats(7.4 * sim::PIXELS_PER_TILE, 260), Outcome::Died);
        assert!(
            attempted > parked,
            "parking in front of a hazard ({parked:.1}) must not beat trying to clear it \
             ({attempted:.1}): a population that is better off standing still never learns \
             to cross anything, and more of the level stops existing for it"
        );
        // The margin has to be more than a rounding error, or the hill is flat at exactly the
        // decision the level is about.
        assert!(
            attempted - parked > 5.0,
            "the hazard crossing has to pay by a real margin, not {:.2}",
            attempted - parked
        );
    }

    /// …but dying is still a cost, and dying *early* is still the worst thing in the file.
    ///
    /// With the survival term gone, the death penalty is the only thing standing between
    /// "reach the frontier" and "reach as much frontier as a suicidal leap happens to touch".
    /// It has to be a real loss against standing still, or a genome that walks into a spike
    /// for two tiles of frontier would be as good as one that survives to the goal.
    #[test]
    fn but_a_death_with_nothing_to_show_for_it_is_still_worse_than_standing_still() {
        let stander = score(&stats(0.0, 600), Outcome::Stuck);
        let suicide = score(&stats(0.0, 30), Outcome::Died);
        assert!(
            suicide < stander,
            "dying without progress ({suicide}) must still be worse than not moving ({stander})"
        );
        assert!((suicide - DEATH_PENALTY).abs() < 1e-3, "{suicide}");
    }

    /// The reason the death penalty is sized the way it is: it has to be smaller than the
    /// smallest thing a hazard crossing buys, or the crossing is never worth attempting.
    #[test]
    fn the_death_penalty_is_smaller_than_the_smallest_hazard_crossing() {
        // One tile of spike, landing one tile past it: two tiles of new rightmost ground.
        let smallest_crossing = 2.0 * sim::PIXELS_PER_TILE;
        assert!(
            -DEATH_PENALTY < smallest_crossing,
            "a death costs {:.1}; clearing the smallest hazard gains {smallest_crossing:.1}, so \
             the attempt has to be worth it at any success rate above {:.0}%",
            -DEATH_PENALTY,
            100.0 * -DEATH_PENALTY / (smallest_crossing - DEATH_PENALTY)
        );
    }

    #[test]
    fn progress_alone_beats_standing_still() {
        let stander = score(&stats(0.0, 600), Outcome::Stuck);
        let mover = score(&stats(64.0, 600), Outcome::Stuck);
        assert!(mover > stander);
    }

    /// The v2 term's one job: hopping must not be free, and must not be punished so hard
    /// that jumping stops being worth it. The two halves are the point — a penalty that
    /// only ever discourages is how a population learns to stand still at a pit.
    #[test]
    fn hopping_in_place_costs_more_than_standing_still_but_a_useful_jump_pays() {
        // 600 frames of hopping is 15 jumps: the exact cadence of a held button before
        // v2, and now strictly worse than standing there.
        let hopper = score(&stats_with_jumps(15, 0.0, 600), Outcome::Stuck);
        let stander = score(&stats(0.0, 600), Outcome::Stuck);
        assert!(
            hopper < stander,
            "15 useless hops ({hopper}) must score below standing still ({stander})"
        );

        // And the cheapest jump that *does* something clears a one-tile gap: 16 pixels of
        // progress against a 2.0 charge.
        let useful = sim::PIXELS_PER_TILE * PROGRESS_PER_PIXEL + JUMP_PENALTY;
        assert!(
            useful > 0.0,
            "clearing a one-tile gap must out-earn the jump that cleared it: {useful}"
        );
        assert!(
            useful > -JUMP_PENALTY * 4.0,
            "with three times the margin the term is a tie-break, not a wall: {useful}"
        );
    }

    fn level() -> Level {
        crate::fixture::flat_runway("fitness-fixture", 40, 38)
    }

    /// Same runway with a three-tile pit at columns 16-18: a constant `run`
    /// policy dies in it, so reaching the flag needs a real policy.
    fn pit_level() -> Level {
        crate::fixture::gap_runway("fitness-pit-fixture", 40, 38, &[(16, 3)])
    }

    #[test]
    fn the_episode_driver_is_deterministic_and_terminates() {
        let level = level();
        let mut rng = rng_from_seed(1);
        let g = Genome::random(1, default_shape(), &mut rng);
        let a = run_episode(&level, &g, &EvalConfig::default(), 1234);
        let b = run_episode(&level, &g, &EvalConfig::default(), 1234);
        assert_eq!(a, b, "same genome + same seed must reproduce exactly");
        assert!(a.outcome.is_finished(), "episodes always terminate");
        assert!(a.frames <= sim::FRAMES_PER_TILE * level.width as u32 + ACTION_REPEAT + 8);
    }

    #[test]
    fn replaying_the_same_genome_with_a_different_seed_may_differ_but_stays_valid() {
        let level = level();
        let mut rng = rng_from_seed(2);
        let g = Genome::random(1, default_shape(), &mut rng);
        let a = run_episode(&level, &g, &EvalConfig::default(), 1);
        let b = run_episode(&level, &g, &EvalConfig::default(), 2);
        assert!(a.fitness.is_finite() && b.fitness.is_finite());
        assert_eq!(a.genome_id, b.genome_id);
    }

    #[test]
    fn the_zero_genome_is_a_free_always_run_policy() {
        // Every output of an all-zeros genome is tanh(0) == 0, and the argmax
        // tie-break picks the lowest index, which is `run`. That is worth pinning:
        // it means evolution starts from "hold right" for free, and it means a
        // flat level is *not* evidence of learning.
        let g = Genome::zeros(0, default_shape());
        let flat = run_episode(&level(), &g, &EvalConfig::default(), 7);
        assert_eq!(
            g.act(&[0.0f32; sim::sensors::N_INPUTS], &mut ActScratch::new()),
            AgentAction::Run
        );
        assert!(flat.won(), "always-run solves a flat runway: {:?}", flat.outcome);

        // With a pit it does not: run straight in and die. This is what makes the
        // pit fixture a real test.
        let pit = run_episode(&pit_level(), &g, &EvalConfig::default(), 7);
        assert!(!pit.won(), "always-run must not survive a pit");
        assert_eq!(pit.outcome, Outcome::Died);
    }

    /// **A checkpoint pays for arriving, and the frontier already pays for flying over.**
    ///
    /// The same frontier twice: one episode stood on the far side of a hazard, the other
    /// only passed over it. `best_x` cannot tell them apart — it is credited for any x the
    /// body reaches, so the leap that ended in the air already banked its distance — and
    /// this term is the half that requires the body to still be on the ground.
    #[test]
    fn a_checkpoint_pays_for_arriving_where_the_frontier_only_pays_for_passing_over() {
        let frontier = 2.0 * sim::PIXELS_PER_TILE;
        let arrived = score(
            &stats_with_checkpoints(1, frontier, 200),
            Outcome::Stuck,
        );
        let flew_over = score(&stats(frontier, 200), Outcome::Stuck);
        assert_eq!(
            arrived - flew_over,
            CHECKPOINT_BONUS,
            "the only difference between the two episodes is where the body ended up"
        );
    }

    /// One tile of progress — legible beside the crossing that earns it, and far below the
    /// frontier it rides on, so it can never become a second objective.
    #[test]
    fn a_checkpoint_is_worth_one_tile_and_cannot_rival_the_frontier() {
        assert_eq!(CHECKPOINT_BONUS, sim::PIXELS_PER_TILE);
        // The smallest hazard crossing pays its two tiles of frontier and this on top.
        let crossing = 2.0 * sim::PIXELS_PER_TILE * PROGRESS_PER_PIXEL + CHECKPOINT_BONUS;
        assert!(
            crossing > -DEATH_PENALTY,
            "a crossing ({crossing}) must out-earn the death it risks ({})",
            -DEATH_PENALTY
        );
        // A hazard field thirteen pads long banks 208 points of checkpoints against a
        // 10 000-point goal: a nudge to the gradient, not a competing objective.
        let whole_field = 13.0 * CHECKPOINT_BONUS;
        assert!(
            whole_field < GOAL_BONUS / 20.0,
            "a whole level of checkpoints ({whole_field}) must stay far under the goal"
        );
        assert!(
            whole_field > 100.0,
            "...and still be big enough to move a selection decision: {whole_field}"
        );
    }

    /// The bonus is per zone and one-shot, so it can never be farmed in place of progress.
    /// The sim owns the counting, but the arithmetic that keeps it safe lives here: a
    /// checkpoint is worth less than the ground it is standing on, and the score is a
    /// function of the count rather than of anything the body can repeat.
    #[test]
    fn checkpoints_cannot_be_farmed_in_place_of_progress() {
        let padded = score(
            &stats_with_checkpoints(2, 0.0, 600),
            Outcome::Stuck,
        );
        let walked = score(&stats(2.0 * sim::PIXELS_PER_TILE, 600), Outcome::Stuck);
        assert_eq!(
            padded, walked,
            "two checkpoints are worth exactly the two tiles of frontier they represent, so \
             standing on them is not a substitute for covering ground"
        );
    }

    #[test]
    fn action_repeat_of_one_still_terminates() {
        let level = level();
        let mut rng = rng_from_seed(3);
        let g = Genome::random(1, default_shape(), &mut rng);
        let cfg = EvalConfig { action_repeat: 1 };
        let r = run_episode(&level, &g, &cfg, 99);
        assert!(r.outcome.is_finished());
    }

    #[test]
    fn a_hand_wired_goal_seeker_reaches_the_flag() {
        // Guards against a sign bug between sensors, action indices and the goal
        // bonus: a genome biased to always choose the highest-scoring action for
        // "run right" solves a flat level. `run` is action index 0, so a genome
        // whose first output dominates walks right and touches the flag.
        let level = level();
        let mut g = Genome::zeros(0, default_shape());
        // Output layer rows are the last `outputs * fan_in` weights. Push row 0
        // (`run`) up by a constant positive bias so argmax always picks it.
        let n_out = g.shape.outputs;
        let (fan_in, _) = g.shape.layer(g.shape.n_layers() - 1);
        let out_w_off = g.shape.n_weights() - n_out * fan_in;
        let out_b_off = g.shape.n_biases() - n_out;
        for c in 0..fan_in {
            g.weights[out_w_off + c] = 0.0;
        }
        g.biases[out_b_off] = 1.0;
        let mut s = ActScratch::new();
        assert_eq!(
            g.act(&[0.0f32; sim::sensors::N_INPUTS], &mut s),
            AgentAction::Run,
            "fixture genome must always choose `run`"
        );
        let r = run_episode(&level, &g, &EvalConfig::default(), 42);
        assert!(
            r.won(),
            "always-run must touch a flat level's flag, got {:?} at x={}",
            r.outcome,
            r.best_x
        );
    }

    /// A win pays for the budget it did not spend, in proportion, and only a win does.
    #[test]
    fn a_win_pays_for_the_time_it_did_not_spend() {
        let cap = budget();
        // Two identical wins, one at the cap and one in half of it.
        let at_the_cap = score(&stats(500.0, cap), Outcome::Won);
        let half_spent = score(&stats(500.0, cap / 2), Outcome::Won);
        let paid = half_spent - at_the_cap;
        assert!(
            (paid - SPEED_BONUS / 2.0).abs() < 1e-3,
            "half the budget unspent is worth half the speed bonus, got {paid}"
        );
        // The term is bounded: a win that spent nothing earns the whole bonus and not a point
        // more, so no episode can outrun the reward by being instantaneous.
        let instant = score(&stats(500.0, 0), Outcome::Won) - at_the_cap;
        assert!((instant - SPEED_BONUS).abs() < 1e-3, "got {instant}");
        // And a *loss* earns nothing for being quick: dying early and dying at the cap score the
        // same, or the fastest route to points would be to be killed, which is the park trap with
        // the sign flipped.
        assert_eq!(
            score(&stats(500.0, cap), Outcome::Died),
            score(&stats(500.0, cap / 4), Outcome::Died),
            "a death must not bank the clock"
        );
        // Which leaves the ordering that matters intact: any win, however slow, still beats any
        // death, however instant, by more than the whole speed bonus is worth.
        assert!(at_the_cap - score(&stats(500.0, 0), Outcome::Died) > 9_000.0);
    }

    /// A quicker win cannot be bought by skipping a checkpoint, which is the trade the term's *size*
    /// has to get right for the reward to be about pace rather than about cutting corners.
    ///
    /// The arithmetic, on the sixty-tile budget every shipped level has (2400 frames): a hundred
    /// frames shaved off a win is `SPEED_BONUS × 100 / 2400` ≈ 8.3 points, and one checkpoint is worth
    /// 16. So a solver that hops over a landing zone to save a sixth of a second comes out behind, and
    /// the term can only be earned by being quick along a route that still collects what the level
    /// pays for. Two hundred frames — a tenth of the budget — is still only 16.7 against one zone.
    #[test]
    fn a_quicker_win_cannot_be_bought_by_skipping_a_checkpoint() {
        let cap = budget();
        let collected = score(&stats_with_checkpoints(1, 500.0, cap - 100), Outcome::Won);
        let skipped = score(&stats_with_checkpoints(0, 500.0, cap - 200), Outcome::Won);
        assert!(
            collected > skipped,
            "skipping a zone for 100 frames paid {skipped:.2} against {collected:.2}"
        );
        // And the bound the whole design rests on: no quantity of saved time can turn a loss into a
        // win, however quickly the loss ended.
        let fastest_loss = score(&stats_with_checkpoints(0, 500.0, 0), Outcome::Died);
        let slowest_win = score(&stats_with_checkpoints(0, 500.0, cap), Outcome::Won);
        assert!(slowest_win - fastest_loss > 9_000.0);
    }

    /// The clock a real episode is paid on is the sim's own frame count against the level's own
    /// budget — not a number the caller supplied, and not one the test supplied either.
    #[test]
    fn the_clock_an_episode_is_scored_on_is_the_sims_own() {
        let level = level();
        let mut g = Genome::zeros(0, default_shape());
        // Always-run: the hand-wired seeker above, which is quick on this flat level.
        let n_out = g.shape.outputs;
        let out_b_off = g.shape.n_biases() - n_out;
        g.biases[out_b_off] = 1.0;

        let mut runner = EpisodeRunner::new(&level, g, &EvalConfig::default(), 42);
        while !runner.is_finished() {
            runner.step();
        }
        let stats = runner.sim().stats().clone();
        let outcome = runner.sim().outcome();
        let r = runner.result();
        assert!(r.won(), "the fixture has to win for the term to be paid at all");

        let budget_frames = sim::frame_budget(&level);
        assert_eq!(stats.budget_frames, budget_frames, "the sim's own budget");
        assert_eq!(r.frames, stats.frames, "the clock is the episode's frame count");
        assert!(
            stats.frames < budget_frames,
            "a win inside the budget is what the term pays for: {} of {}",
            stats.frames,
            budget_frames
        );

        // The recorded fitness is the recorded stats scored once, and taking the speed term out
        // of it costs exactly the unspent fraction of the bonus — no more, no less. Which is what
        // says the term is *in* the number a champion file will advertise, rather than only in a
        // synthetic `score` call a test made up.
        assert_eq!(r.fitness, score(&stats, outcome));
        let unspent_terms = stats.progress_pixels * PROGRESS_PER_PIXEL
            + stats.jumps as f32 * JUMP_PENALTY
            + stats.checkpoints as f32 * CHECKPOINT_BONUS
            + stats.route_hops_closed as f32 * ROUTE_HOP_BONUS
            + GOAL_BONUS;
        let paid = r.fitness - unspent_terms;
        let expected = SPEED_BONUS * (budget_frames - stats.frames) as f32 / budget_frames as f32;
        assert!(
            (paid - expected).abs() < 1e-2,
            "the win was paid {paid} for its clock, expected {expected} from {} of {}",
            stats.frames,
            budget_frames
        );
    }

}
