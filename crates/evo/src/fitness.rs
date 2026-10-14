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
pub const FITNESS_VERSION: u32 = 5;

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
    if outcome == Outcome::Died {
        f += DEATH_PENALTY;
    }
    if outcome == Outcome::Won {
        f += GOAL_BONUS;
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
    pub fitness: f32,
    /// Mean fitness over all trials; equal to `fitness` when `trials == 1`.
    ///
    /// Selection ranks on this. A single episode is noisy evidence about a genome,
    /// but it is also the only number a third party can reproduce from one seed,
    /// so the two are kept apart instead of being conflated: without this split, a
    /// champion trained with `--trials 4` would advertise an average and then fail
    /// its own replay check.
    pub mean_fitness: f32,
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
    pub death_pos: Option<(f32, f32)>,
    pub seed: u64,
    /// Hash of the decision trace and final state. Champion round-trip tests
    /// compare this, so a replay that diverges anywhere is caught.
    pub trajectory_hash: u64,
}

impl EpisodeResult {
    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
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
            outcome,
            frames: stats.frames,
            progress_pixels: stats.progress_pixels,
            best_x: stats.best_x,
            stomps: stats.stomps,
            jumps: stats.jumps,
            checkpoints: stats.checkpoints,
            death_pos: stats.death_pos,
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

    fn stats(progress_pixels: f32, frames: u32) -> EpisodeStats {
        EpisodeStats {
            frames,
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

}
