//! **Hybrid execution: a fast policy with model-based recovery.**
//!
//! A learned policy is cheap and expressive, but a single bad action can destroy an otherwise
//! good route. The timed planner already knows how to search from an arbitrary [`sim::Sim`]
//! snapshot, so execution can use the policy until its next block is unsafe and then hand the
//! *actual* state to the planner. This is deliberately closed-loop: recovery does not replay the
//! original route by frame number and does not pretend the policy reached the state it was meant to.

use sim::physics::AgentAction;
use sim::{Level, ObservationScratch, Outcome, Sim};

use crate::fitness::EvalConfig;
use crate::genome::{ActScratch, Genome};

/// Limits for hybrid execution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HybridConfig {
    /// Planner budget for a recovery route.
    pub plan: sim::plan::PlanConfig,
    /// Replan after this many policy blocks without new progress.
    pub replan_after: u32,
    /// Maximum recovery searches in one episode.
    pub max_replans: u32,
    /// Hard decision cap independent of the simulator's frame cap.
    pub max_decisions: u32,
}

impl Default for HybridConfig {
    fn default() -> Self {
        HybridConfig {
            plan: sim::plan::PlanConfig {
                depth: 48,
                ..sim::plan::PlanConfig::default()
            },
            replan_after: 8,
            max_replans: 32,
            max_decisions: 10_000,
        }
    }
}

impl HybridConfig {
    pub fn sanitized(mut self) -> Self {
        self.plan = self.plan.sanitized();
        self.replan_after = self.replan_after.clamp(1, 600);
        self.max_replans = self.max_replans.clamp(1, 256);
        self.max_decisions = self.max_decisions.clamp(1, 100_000);
        self
    }
}

/// What a hybrid episode did.
#[derive(Clone, Debug, PartialEq)]
pub struct HybridReport {
    /// One action per executed decision block.
    pub actions: Vec<AgentAction>,
    pub outcome: Outcome,
    pub frames: u32,
    pub best_x: f32,
    pub route_hops_closed: u32,
    /// Number of searches started from the policy's current state.
    pub replans: u32,
    pub policy_blocks: u32,
    pub planned_blocks: u32,
    pub death_cause: Option<&'static str>,
    pub note: String,
}

impl HybridReport {
    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
    }
}

/// Run `genome` with planner recovery.
///
/// Before committing a policy action, the block is simulated on a clone. If it would die, or if
/// the policy has stopped making progress, a timed route is searched from the current live state.
/// A route is followed at the planner's decision cadence and can itself be replaced by another
/// route if a future physics change makes it fail. This makes the function useful both for a
/// player-facing solver and for measuring how much a policy needs the deliberate layer.
pub fn play(
    level: &Level,
    genome: &Genome,
    eval: &EvalConfig,
    seed: u64,
    cfg: HybridConfig,
) -> HybridReport {
    let cfg = cfg.sanitized();
    let repeat = eval.action_repeat.max(1);
    let mut sim = Sim::new(level, seed);
    let mut scratch = ActScratch::new();
    let mut observation = ObservationScratch::new();
    let mut actions = Vec::new();
    let mut recovery: Option<Vec<AgentAction>> = None;
    let mut recovery_at = 0usize;
    let mut replans = 0u32;
    let mut policy_blocks = 0u32;
    let mut planned_blocks = 0u32;
    let mut no_progress = 0u32;
    let mut last_progress = sim.stats().progress_pixels;
    let mut last_hops = sim.stats().route_hops_closed;

    while !sim.outcome().is_finished() && actions.len() < cfg.max_decisions as usize {
        if let Some(route) = &recovery {
            if recovery_at >= route.len() {
                recovery = None;
                recovery_at = 0;
                continue;
            }
            let action = route[recovery_at];
            recovery_at += 1;
            // The planner's route is compiled on the sim's fixed decision cadence. A policy may
            // have a different experimental cadence (or a commitment head), but recovery actions
            // still belong to the planner's own block grid.
            step_block(&mut sim, action, sim::DECISION_TICKS);
            actions.push(action);
            planned_blocks += 1;
            update_progress(&sim, &mut last_progress, &mut last_hops, &mut no_progress);
            continue;
        }

        let obs = sim.observation_into(&mut observation);
        let (action, policy_repeat) = if genome.shape.commitment {
            genome.act_with_commitment(&obs, &mut scratch)
        } else {
            (genome.act(&obs, &mut scratch), repeat)
        };
        // Look ahead one policy block. A policy is allowed to be imperfect, but it should not be
        // allowed to knowingly step into a lethal state when a recovery plan can be searched first.
        let mut probe = sim.clone();
        step_block(&mut probe, action, policy_repeat);
        let stalled = no_progress >= cfg.replan_after;
        let unexpected_fall =
            sim.player().grounded && !probe.player().grounded && !action.wants_jump();
        let unsafe_block = probe.outcome() == Outcome::Died
            || probe.outcome() == Outcome::Stuck
            || unexpected_fall;
        if (unsafe_block || stalled) && replans < cfg.max_replans {
            if let Some(route) = sim::plan::solve_from(level, sim.clone(), seed, cfg.plan) {
                recovery = Some(route.actions);
                recovery_at = 0;
                replans += 1;
                no_progress = 0;
                continue;
            }
            // No route from this state: commit the policy action once, so the controller remains
            // a useful fallback on levels outside the planner's action envelope.
            no_progress = 0;
        }

        step_block(&mut sim, action, policy_repeat);
        actions.push(action);
        policy_blocks += 1;
        update_progress(&sim, &mut last_progress, &mut last_hops, &mut no_progress);
    }

    let stats = sim.stats();
    let note = if sim.outcome() == Outcome::Won {
        format!(
            "hybrid policy won in {} frames ({} policy blocks, {} planned blocks, {} replans)",
            stats.frames, policy_blocks, planned_blocks, replans
        )
    } else {
        format!(
            "hybrid controller ended as {:?} after {} frames at x {:.2} ({} replans)",
            sim.outcome(),
            stats.frames,
            stats.best_x,
            replans
        )
    };
    HybridReport {
        actions,
        outcome: sim.outcome(),
        frames: stats.frames,
        best_x: stats.best_x,
        route_hops_closed: stats.route_hops_closed,
        replans,
        policy_blocks,
        planned_blocks,
        death_cause: sim.death_cause().map(|cause| cause.as_str()),
        note,
    }
}

fn step_block(sim: &mut Sim, action: AgentAction, repeat: u32) {
    for _ in 0..repeat {
        if sim.outcome().is_finished() {
            break;
        }
        sim.step(action);
    }
}

fn update_progress(sim: &Sim, last_progress: &mut f32, last_hops: &mut u32, no_progress: &mut u32) {
    let progress = sim.stats().progress_pixels;
    let hops = sim.stats().route_hops_closed;
    if progress > *last_progress || hops > *last_hops {
        *last_progress = progress;
        *last_hops = hops;
        *no_progress = 0;
    } else {
        *no_progress = no_progress.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{default_shape, Genome};

    fn level() -> Level {
        Level::from_json_str(include_str!("../../../levels/l2-first-gap.json"))
            .expect("the shipped gap level parses")
    }

    #[test]
    fn a_policy_that_walks_into_the_gap_is_recovered_by_a_plan() {
        let genome = Genome::zeros(0, default_shape());
        let report = play(
            &level(),
            &genome,
            &EvalConfig::default(),
            1,
            HybridConfig::default(),
        );
        assert!(report.won(), "{}", report.note);
        assert!(report.replans > 0, "the recovery path was not used");
        assert!(report.planned_blocks > 0);
    }
}
