//! **Fair hard-level generation.**
//!
//! Difficulty is selected only after the same authorities that certify authored levels have
//! spoken. Candidates are allowed to be nasty, but a candidate is returned only when the timed
//! planner wins it with useful timing slack and the route survives a small held-out phase battery.
//! This makes the generator a source of demanding levels rather than a factory for accidental
//! unwinnable files.

use serde::{Deserialize, Serialize};

use crate::plan::PlanConfig;
use crate::practice;
use crate::route_plan::{self, RoutePlan};
use crate::{Level, Outcome, Sim, Tile, DECISION_TICKS};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChallengeConfig {
    pub seed: u64,
    pub candidates: usize,
    pub base_tier: usize,
    pub planner: PlanConfig,
    pub min_margin_frames: u32,
    pub phase_seeds: u32,
}

impl Default for ChallengeConfig {
    fn default() -> Self {
        ChallengeConfig {
            seed: 0xC4A1_1E6E,
            candidates: 64,
            base_tier: 3,
            planner: PlanConfig {
                depth: 64,
                ..PlanConfig::default()
            },
            min_margin_frames: 120,
            phase_seeds: 4,
        }
    }
}

impl ChallengeConfig {
    pub fn sanitized(self) -> Self {
        ChallengeConfig {
            seed: self.seed,
            candidates: self.candidates.clamp(1, 512),
            base_tier: self.base_tier.min(3),
            planner: self.planner.sanitized(),
            min_margin_frames: self.min_margin_frames,
            phase_seeds: self.phase_seeds.clamp(1, 16),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FairnessCertificate {
    pub planner_frames: u32,
    pub budget_frames: u32,
    pub tightest_margin_frames: u32,
    pub phase_seeds: u32,
    pub phase_wins: u32,
    pub mechanic_count: u32,
    pub route_hops: u32,
}

impl FairnessCertificate {
    pub fn fair(&self) -> bool {
        self.tightest_margin_frames > 0 && self.phase_wins == self.phase_seeds
    }

    pub fn difficulty(&self) -> f32 {
        let timing = self.planner_frames as f32 / self.budget_frames.max(1) as f32;
        timing + self.route_hops as f32 * 0.06 + self.mechanic_count as f32 * 0.025
    }
}

#[derive(Clone, Debug)]
pub struct ChallengeLevel {
    pub level: Level,
    pub plan: RoutePlan,
    pub fairness: FairnessCertificate,
    pub attempts: usize,
}

/// Generate the hardest fair candidate found in a deterministic candidate budget.
pub fn generate(config: ChallengeConfig) -> Result<ChallengeLevel, String> {
    let config = config.sanitized();
    let mut best: Option<ChallengeLevel> = None;
    for attempt in 0..config.candidates {
        let index = (config.seed as usize)
            .wrapping_add(config.base_tier * practice::ARCHETYPES.len())
            .wrapping_add(attempt)
            % practice::SUITE_LEN;
        let (mut level, _) = practice::generate(index);
        decorate(&mut level, config.seed, attempt);
        level.name = format!("challenge-{:016x}-{:03}", config.seed, attempt);
        let Ok(plan) = route_plan::compile(&level, config.seed, config.planner) else {
            continue;
        };
        if plan.tightest_margin() < config.min_margin_frames {
            continue;
        }
        let phase_wins = (0..config.phase_seeds)
            .filter(|phase| replay_plan(&level, &plan, config.seed.wrapping_add(*phase as u64)))
            .count() as u32;
        let fairness = FairnessCertificate {
            planner_frames: plan.frames,
            budget_frames: plan.budget_frames,
            tightest_margin_frames: plan.tightest_margin(),
            phase_seeds: config.phase_seeds,
            phase_wins,
            mechanic_count: mechanic_count(&level),
            route_hops: plan.hops,
        };
        if !fairness.fair() {
            continue;
        }
        let candidate = ChallengeLevel {
            level,
            plan,
            fairness,
            attempts: attempt + 1,
        };
        let better = best
            .as_ref()
            .is_none_or(|held| candidate.fairness.difficulty() > held.fairness.difficulty());
        if better {
            best = Some(candidate);
        }
    }
    best.ok_or_else(|| {
        format!(
            "no fair planner-certified level found in {} candidates (minimum margin {} frames)",
            config.candidates, config.min_margin_frames
        )
    })
}

fn replay_plan(level: &Level, plan: &RoutePlan, seed: u64) -> bool {
    let mut sim = Sim::new(level, seed);
    for action in &plan.actions {
        for _ in 0..DECISION_TICKS {
            if sim.outcome().is_finished() {
                break;
            }
            sim.step(*action);
        }
        if sim.outcome().is_finished() {
            break;
        }
    }
    sim.outcome() == Outcome::Won
}

fn mechanic_count(level: &Level) -> u32 {
    let spikes = level
        .to_glyph_rows()
        .iter()
        .map(|row| row.chars().filter(|tile| *tile == '^').count() as u32)
        .sum::<u32>();
    spikes
        + level.walkers.len() as u32
        + level.chasers.len() as u32
        + level.flyers.len() as u32
        + level.turrets.len() as u32
        + level.platforms.len() as u32
        + level.ropes.len() as u32
        + level.spike_balls.len() as u32
        + level.rocks.len() as u32
        + level.blocks.len() as u32
}

fn surface_row(level: &Level, x: i32) -> Option<i32> {
    (1..level.height).find(|row| level.is_solid(x, *row) && !level.is_solid(x, *row - 1))
}

fn decorate(level: &mut Level, seed: u64, attempt: usize) {
    let width = level.width.max(16);
    let x = 6 + split(seed, attempt as u64, 1) as i32 % (width - 12);
    let surface = surface_row(level, x).unwrap_or(level.height - 2);
    match attempt % 6 {
        0 => {
            let _ = level.add_walker_tile(x, surface - 1);
            let _ = level.add_turret_tile((x + 5).min(width - 3), surface - 1, -1);
        }
        1 => {
            let _ = level.add_chaser_tile(x, surface - 1);
            let _ = level.add_flyer_tile((x + 4).min(width - 3), (surface - 5).max(1), 3);
        }
        2 => {
            let _ = level.add_turret_tile(x, surface - 1, if attempt & 1 == 0 { -1 } else { 1 });
            let _ = level.add_walker_tile((x + 6).min(width - 3), surface - 1);
        }
        3 => {
            let _ = level.add_flyer_tile(x, (surface - 5).max(1), 4);
            let _ = level.add_spike_ball_tile((x + 3).min(width - 3), 1);
        }
        4 => {
            let _ = level.add_chaser_tile(x, surface - 1);
            if level.tile(x, surface - 1) == Some(Tile::Empty) {
                let _ = level.set_tile(x, surface - 1, Tile::Spike);
            }
        }
        _ => {
            let _ = level.add_walker_tile(x, surface - 1);
            let _ = level.add_flyer_tile((x + 3).min(width - 3), (surface - 4).max(1), 2);
        }
    }
}

fn split(seed: u64, salt: u64, extra: u64) -> u64 {
    let mut z = seed
        .wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(extra.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z ^ (z >> 27)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_challenges_are_planned_and_have_phase_margin() {
        let generated = generate(ChallengeConfig {
            candidates: 12,
            phase_seeds: 2,
            min_margin_frames: 60,
            ..ChallengeConfig::default()
        })
        .expect("the deterministic challenge budget finds a fair level");
        assert!(generated.fairness.fair());
        assert!(generated.fairness.tightest_margin_frames >= 60);
        assert_eq!(
            generated.fairness.phase_wins,
            generated.fairness.phase_seeds
        );
        assert_eq!(
            generated.plan.level_hash,
            generated.level.content_hash_string()
        );
        assert!(!generated.plan.actions.is_empty());
    }
}
