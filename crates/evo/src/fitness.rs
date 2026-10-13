//! Fitness function v1 (§7.4) and the one episode driver everything shares.
//!
//! **Versioned** (§8): any change to a reward term, to the action-repeat cadence,
//! or to the termination rules bumps [`FITNESS_VERSION`]. Champion files record
//! it, and a champion is only loadable when the version matches.
//!
//! Anti-exploit rule (from "Surprising Creativity of Digital Evolution"): reward
//! task progress plus survival, never a proxy. No term rewards jump height,
//! airtime, stomps, or distance travelled backwards — agents farm proxies.

use sim::hash::StateHasher;
use sim::{EpisodeStats, Level, Outcome, Sim};

use crate::genome::{ActScratch, Genome};

/// Fitness layout version. Bump on *any* change below (§8).
pub const FITNESS_VERSION: u32 = 1;

/// Physics ticks between network queries (§7.3). 60 Hz sim → a decision every
/// ~67 ms; the chosen action repeats in between. Exposed as a tuning knob; the
/// plan's range is 3–6.
pub const ACTION_REPEAT: u32 = 4;

/// Fitness per new rightmost pixel of ground covered (§7.4.1).
pub const PROGRESS_PER_PIXEL: f32 = 1.0;
/// Fitness per frame survived (§7.4.2). Deliberately small: enough to prefer
/// living over dying, not enough to reward camping.
pub const SURVIVAL_PER_FRAME: f32 = 0.05;
/// Fitness on dying (§7.4.3).
pub const DEATH_PENALTY: f32 = -120.0;
/// Fitness on touching the flag (§7.4.4). Must dominate any progress-only local
/// optimum, which is why it is five figures while a full-width level's progress
/// score is four.
pub const GOAL_BONUS: f32 = 10_000.0;

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
pub fn score(stats: &EpisodeStats, outcome: Outcome) -> f32 {
    let mut f = stats.progress_pixels * PROGRESS_PER_PIXEL;
    f += stats.frames as f32 * SURVIVAL_PER_FRAME;
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
    pub fitness: f32,
    pub outcome: Outcome,
    pub frames: u32,
    pub progress_pixels: f32,
    pub best_x: f32,
    pub stomps: u32,
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
    let repeat = cfg.action_repeat.max(1);
    let mut sim = Sim::new(level, seed);
    let mut scratch = ActScratch::new();
    let mut trace = StateHasher::new();
    let mut action = sim::physics::AgentAction::Walk;

    // `max_frames` is the sim's own cap; the extra slack is a belt-and-braces
    // guard against a future change that forgets to terminate.
    let guard = sim.max_frames() + repeat + 8;
    let mut iterations = 0u32;
    while !sim.outcome().is_finished() {
        let frames = sim.stats().frames;
        if frames % repeat == 0 {
            action = genome.act(&sim.observation(), &mut scratch);
            trace.u32(frames);
            trace.u8(action.index() as u8);
        }
        sim.step(action);
        iterations += 1;
        if iterations >= guard {
            break;
        }
    }

    trace.u64(sim.state_hash());
    let outcome = sim.outcome();
    let stats = sim.stats();
    EpisodeResult {
        genome_id: genome.id,
        fitness: score(stats, outcome),
        outcome,
        frames: stats.frames,
        progress_pixels: stats.progress_pixels,
        best_x: stats.best_x,
        stomps: stats.stomps,
        death_pos: stats.death_pos,
        seed,
        trajectory_hash: trace.finish(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{default_shape, Genome};
    use sim::physics::AgentAction;
    use sim::rng::rng_from_seed;
    use sim::EpisodeStats;

    fn stats(progress_pixels: f32, frames: u32) -> EpisodeStats {
        EpisodeStats {
            frames,
            progress_pixels,
            best_x: 0.0,
            no_progress_frames: 0,
            stomps: 0,
            death_pos: None,
        }
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

    #[test]
    fn dying_costs_more_than_the_survival_bonus_can_repay() {
        // 600 frames of survival is 30 fitness; a death must be a net loss.
        let alive = score(&stats(0.0, 600), Outcome::Stuck);
        let dead = score(&stats(0.0, 600), Outcome::Died);
        assert!(dead < alive);
        assert!((alive - dead + DEATH_PENALTY).abs() < 1e-3, "{alive} {dead}");
    }

    #[test]
    fn progress_alone_beats_standing_still() {
        let stander = score(&stats(0.0, 600), Outcome::Stuck);
        let mover = score(&stats(64.0, 600), Outcome::Stuck);
        assert!(mover > stander);
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
