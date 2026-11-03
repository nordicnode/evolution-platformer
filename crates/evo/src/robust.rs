//! **Robust policy evaluation.**
//!
//! A single winning episode is a demonstration, not a reliable solver. This module gives tooling
//! a fixed, reproducible battery of held-out worlds and reports the policy's worst case alongside
//! its mean. Training can keep its cheap generation loop; certification and the player-facing
//! score command can ask the stronger question explicitly.

use sim::physics::AgentAction;
use sim::rng::episode_seed;
use sim::Level;

use crate::fitness::{run_episode, EpisodeResult, EpisodeRunner, EvalConfig};
use crate::Genome;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobustConfig {
    /// Number of held-out episodes. Clamped to a useful finite range.
    pub episodes: u32,
    pub action_repeat: u32,
}

impl Default for RobustConfig {
    fn default() -> Self {
        RobustConfig {
            episodes: 24,
            action_repeat: crate::ACTION_REPEAT,
        }
    }
}

impl RobustConfig {
    pub fn sanitized(mut self) -> Self {
        self.episodes = self.episodes.clamp(1, 256);
        self.action_repeat = self.action_repeat.clamp(1, 32);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RobustReport {
    pub episodes: u32,
    pub wins: u32,
    pub mean_fitness: f32,
    pub worst_fitness: f32,
    pub fastest_win: Option<u32>,
    pub slowest_win: Option<u32>,
    pub worst_best_x: f32,
    pub results: Vec<EpisodeResult>,
}

impl RobustReport {
    pub fn solved(&self) -> bool {
        self.episodes > 0 && self.wins == self.episodes
    }

    pub fn win_rate(&self) -> f32 {
        if self.episodes == 0 {
            0.0
        } else {
            self.wins as f32 / self.episodes as f32
        }
    }
}

/// Evaluate a genome on fixed held-out seeds that no generation can issue.
pub fn evaluate(
    level: &Level,
    genome: &Genome,
    master_seed: u64,
    cfg: RobustConfig,
) -> RobustReport {
    let cfg = cfg.sanitized();
    let eval = EvalConfig {
        action_repeat: cfg.action_repeat,
        cut_stuck_frames: 0,
    };
    let mut results = Vec::with_capacity(cfg.episodes as usize);
    for i in 0..cfg.episodes {
        let seed = episode_seed(master_seed, u32::MAX, genome.id, i);
        results.push(run_episode(level, genome, &eval, seed));
    }
    let wins = results.iter().filter(|r| r.won()).count() as u32;
    let sum = results.iter().map(|r| r.fitness).sum::<f32>();
    let worst_fitness = results
        .iter()
        .map(|r| r.fitness)
        .fold(f32::INFINITY, f32::min);
    let worst_best_x = results
        .iter()
        .map(|r| r.best_x)
        .fold(f32::INFINITY, f32::min);
    let mut win_frames: Vec<u32> = results
        .iter()
        .filter(|r| r.won())
        .map(|r| r.frames)
        .collect();
    win_frames.sort_unstable();
    RobustReport {
        episodes: cfg.episodes,
        wins,
        mean_fitness: sum / cfg.episodes as f32,
        worst_fitness,
        fastest_win: win_frames.first().copied(),
        slowest_win: win_frames.last().copied(),
        worst_best_x,
        results,
    }
}

/// A deterministic control fault used for near-miss validation. These are not training actions;
/// they answer whether a certified policy has any recovery margin around a decision boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disturbance {
    MissJump,
    Brake,
    Reverse,
    Delay,
}

impl Disturbance {
    pub const ALL: [Disturbance; 4] = [
        Disturbance::MissJump,
        Disturbance::Brake,
        Disturbance::Reverse,
        Disturbance::Delay,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Disturbance::MissJump => "miss-jump",
            Disturbance::Brake => "brake",
            Disturbance::Reverse => "reverse",
            Disturbance::Delay => "delay",
        }
    }

    fn action(self) -> AgentAction {
        match self {
            Disturbance::MissJump | Disturbance::Delay => AgentAction::Walk,
            Disturbance::Brake => AgentAction::Brake,
            Disturbance::Reverse => AgentAction::Retreat,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdversarialConfig {
    pub episodes: u32,
    pub action_repeat: u32,
    /// Decision boundaries tested per disturbance and episode.
    pub cases_per_episode: u32,
    /// Minimum disturbed win rate a solver contract may call robust.
    pub required_win_rate: f32,
}

impl Default for AdversarialConfig {
    fn default() -> Self {
        AdversarialConfig {
            episodes: 8,
            action_repeat: crate::ACTION_REPEAT,
            cases_per_episode: 3,
            required_win_rate: 0.75,
        }
    }
}

impl AdversarialConfig {
    pub fn sanitized(self) -> Self {
        AdversarialConfig {
            episodes: self.episodes.clamp(1, 64),
            action_repeat: self.action_repeat.clamp(1, 32),
            cases_per_episode: self.cases_per_episode.clamp(1, 8),
            required_win_rate: self.required_win_rate.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdversarialCase {
    pub disturbance: Disturbance,
    pub seed: u64,
    pub decision: u32,
    /// Whether the injected fault was reached before the episode ended. The schedule is chosen
    /// from a clean rollout so a short level is still tested at a real decision boundary rather
    /// than being counted as a quietly unperturbed win.
    pub applied: bool,
    pub result: EpisodeResult,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdversarialReport {
    pub cases: u32,
    pub wins: u32,
    pub win_rate: f32,
    pub results: Vec<AdversarialCase>,
}

impl AdversarialReport {
    pub fn robust(&self, required_win_rate: f32) -> bool {
        self.cases > 0 && self.win_rate >= required_win_rate.clamp(0.0, 1.0)
    }
}

/// Replay a policy while injecting fixed, labelled action faults at decision boundaries. The phase
/// seeds and disturbance schedule are stable, so a report can be compared across training arms.
pub fn evaluate_adversarial(
    level: &Level,
    genome: &Genome,
    master_seed: u64,
    cfg: AdversarialConfig,
) -> AdversarialReport {
    let cfg = cfg.sanitized();
    let eval = EvalConfig {
        action_repeat: cfg.action_repeat,
        cut_stuck_frames: 0,
    };
    let mut results = Vec::new();
    for episode in 0..cfg.episodes {
        let seed = episode_seed(master_seed, u32::MAX - 1, genome.id, episode);
        let mut baseline = EpisodeRunner::new(level, genome.clone(), &eval, seed);
        let mut baseline_decisions = 0u32;
        while !baseline.is_finished() {
            if baseline.step_with_override(None) {
                baseline_decisions += 1;
            }
        }
        let decision_slots = baseline_decisions.max(1);
        for (kind_index, disturbance) in Disturbance::ALL.iter().enumerate() {
            for case in 0..cfg.cases_per_episode {
                // Prefer a non-opening boundary when the clean episode has one, but always choose
                // an index the clean rollout actually reached. Before this, fixed offsets such as
                // decision 8 made every fault a no-op on short levels while still awarding the
                // case as a disturbed win.
                let decision = if decision_slots == 1 {
                    0
                } else {
                    1 + (case
                        .saturating_mul(17)
                        .saturating_add(episode)
                        .saturating_add(kind_index as u32)
                        % (decision_slots - 1))
                };
                let mut runner = EpisodeRunner::new(level, genome.clone(), &eval, seed);
                let mut decisions = 0u32;
                let mut applied = false;
                while !runner.is_finished() {
                    let at_boundary = decisions == decision;
                    let override_action = at_boundary.then(|| disturbance.action());
                    if runner.step_with_override(override_action) {
                        applied |= at_boundary;
                        decisions += 1;
                    }
                }
                results.push(AdversarialCase {
                    disturbance: *disturbance,
                    seed,
                    decision,
                    applied,
                    result: runner.result(),
                });
            }
        }
    }
    // A future scheduler change must not turn an un-applied fault into a free win. Keep every
    // case for diagnostics, but only applied faults enter the certification denominator.
    let cases = results.iter().filter(|case| case.applied).count() as u32;
    let wins = results
        .iter()
        .filter(|case| case.applied && case.result.won())
        .count() as u32;
    AdversarialReport {
        cases,
        wins,
        win_rate: if cases == 0 {
            0.0
        } else {
            wins as f32 / cases as f32
        },
        results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_shape, Genome};

    #[test]
    fn robust_evaluation_is_fixed_and_reports_all_wins() {
        let level = Level::from_json_str(include_str!("../../../levels/l1-flat-run.json"))
            .expect("the flat level parses");
        let genome = Genome::zeros(0, default_shape());
        let cfg = RobustConfig {
            episodes: 4,
            action_repeat: 4,
        };
        let a = evaluate(&level, &genome, 9, cfg);
        let b = evaluate(&level, &genome, 9, cfg);
        assert_eq!(a, b);
        assert_eq!(a.episodes, 4);
        assert_eq!(a.wins, 4);
        assert!(a.solved());
    }

    #[test]
    fn adversarial_cases_are_fixed_and_record_the_disturbance_boundary() {
        let level = Level::from_json_str(include_str!("../../../levels/l1-flat-run.json"))
            .expect("the flat level parses");
        let genome = Genome::zeros(0, default_shape());
        let cfg = AdversarialConfig {
            episodes: 1,
            cases_per_episode: 1,
            ..AdversarialConfig::default()
        };
        let a = evaluate_adversarial(&level, &genome, 12, cfg);
        let b = evaluate_adversarial(&level, &genome, 12, cfg);
        assert_eq!(a, b);
        assert_eq!(a.cases, Disturbance::ALL.len() as u32);
        assert!(
            a.results.iter().all(|case| case.applied),
            "every short-level fault must reach a real decision boundary"
        );
    }
}
