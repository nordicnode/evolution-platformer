//! **Replay presentation data.**
//!
//! A winning result is more useful when a viewer can see the intended route, the learned route, and
//! the exact decisions where they diverged. This module builds a portable JSON report; renderers
//! can consume the same moments without re-running a policy or guessing which frame mattered.

use serde::{Deserialize, Serialize};
use sim::physics::AgentAction;
use sim::route_plan::RoutePlan;
use sim::{Level, DECISION_TICKS};

use crate::discovery::{detect, Discovery};
use crate::fitness::{run_episode, EvalConfig};
use crate::genome::Genome;
use crate::tas::{route_from_genome, TasRoute};
use crate::variants::{explore, VariantConfig, VariantReport};

pub const SHOWCASE_FORMAT: &str = "evo-replay-showcase";
pub const SHOWCASE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShowcaseConfig {
    pub planner: sim::plan::PlanConfig,
    pub variants: VariantConfig,
}

impl Default for ShowcaseConfig {
    fn default() -> Self {
        ShowcaseConfig {
            planner: sim::plan::PlanConfig {
                depth: 64,
                ..sim::plan::PlanConfig::default()
            },
            variants: VariantConfig::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionMoment {
    pub block: u32,
    pub frame: u32,
    pub x: f32,
    pub y: f32,
    pub planned: Option<AgentAction>,
    pub learned: Option<AgentAction>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShowcaseReport {
    pub format: String,
    pub version: u32,
    pub level_name: String,
    pub level_hash: String,
    pub seed: u64,
    pub plan: Option<RoutePlan>,
    pub learned: TasRoute,
    pub moments: Vec<DecisionMoment>,
    pub discoveries: Vec<Discovery>,
    pub variants: VariantReport,
}

impl ShowcaseReport {
    pub fn to_json_pretty(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot encode replay showcase: {e}"))
    }

    pub fn won(&self) -> bool {
        self.learned.won()
    }
}

/// Build a presentation report from a learned genome. Planning remains optional: an evolved route
/// can win through a mechanic outside the planner's route graph, and the report preserves that as
/// a learned-only solution rather than hiding it.
pub fn build(
    level: &Level,
    genome: &Genome,
    eval: &EvalConfig,
    seed: u64,
    cfg: ShowcaseConfig,
) -> ShowcaseReport {
    let plan = sim::route_plan::compile(level, seed, cfg.planner).ok();
    let learned = route_from_genome(level, genome, eval, seed);
    let result = run_episode(level, genome, eval, seed);
    let discoveries = detect(level, genome, &result, eval, 0, true);
    let moments = moments(level, &learned, plan.as_ref());
    let variants = explore(level, &learned, cfg.variants);
    ShowcaseReport {
        format: SHOWCASE_FORMAT.into(),
        version: SHOWCASE_VERSION,
        level_name: level.name.clone(),
        level_hash: level.content_hash_string(),
        seed,
        plan,
        learned,
        moments,
        discoveries,
        variants,
    }
}

fn moments(level: &Level, learned: &TasRoute, plan: Option<&RoutePlan>) -> Vec<DecisionMoment> {
    let Some(plan) = plan else {
        return Vec::new();
    };
    let mut sim = sim::Sim::new(level, learned.seed);
    let blocks = plan
        .actions
        .len()
        .max(learned.actions.len().div_ceil(DECISION_TICKS as usize));
    let mut out = Vec::new();
    for block in 0..blocks {
        if sim.outcome().is_finished() {
            break;
        }
        let planned = plan.actions.get(block).copied();
        let learned_action = learned
            .actions
            .get(block * DECISION_TICKS as usize)
            .copied();
        if planned != learned_action {
            out.push(DecisionMoment {
                block: block as u32,
                frame: sim.stats().frames,
                x: sim.player().x,
                y: sim.player().y,
                planned,
                learned: learned_action,
                reason: reason(planned, learned_action),
            });
        }
        for tick in 0..DECISION_TICKS {
            let index = block * DECISION_TICKS as usize + tick as usize;
            let action = learned
                .actions
                .get(index)
                .copied()
                .unwrap_or(AgentAction::Walk);
            if sim.outcome().is_finished() {
                break;
            }
            sim.step(action);
        }
    }
    out
}

fn reason(planned: Option<AgentAction>, learned: Option<AgentAction>) -> String {
    match (planned, learned) {
        (Some(plan), Some(student)) if plan.wants_jump() && !student.wants_jump() => {
            "learned route released the jump at the planned take-off".into()
        }
        (Some(plan), Some(student)) if !plan.wants_jump() && student.wants_jump() => {
            "learned route pressed jump before the intended crossing".into()
        }
        (Some(_), Some(_)) => "learned route changed its action or gear".into(),
        (Some(_), None) => "learned route ended before the intended route".into(),
        (None, Some(_)) => "learned route continued after the intended route".into(),
        (None, None) => "both routes ended".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{default_shape, Genome};

    #[test]
    fn showcase_is_replayable_and_names_decision_divergence() {
        let level = Level::from_json_str(include_str!("../../../levels/l1-flat-run.json"))
            .expect("the flat level parses");
        let genome = Genome::zeros(0, default_shape());
        let report = build(
            &level,
            &genome,
            &EvalConfig::default(),
            1,
            ShowcaseConfig {
                variants: VariantConfig {
                    candidates: 12,
                    keep: 3,
                    ..VariantConfig::default()
                },
                ..ShowcaseConfig::default()
            },
        );
        assert_eq!(report.format, SHOWCASE_FORMAT);
        assert!(report.learned.won());
        let json = report.to_json_pretty().expect("showcase serializes");
        assert!(json.contains("moments"));
        assert!(report
            .moments
            .iter()
            .all(|moment| moment.reason.chars().count() > 3));
        assert_eq!(
            report.learned.final_state_hash,
            crate::tas::play(&level, 1, &report.learned.actions).final_state_hash
        );
        assert_eq!(
            report.learned.frames,
            crate::tas::play(&level, 1, &report.learned.actions).frames
        );
        assert_eq!(report.learned.kind, crate::tas::TasKind::Unpolished);
        assert_eq!(report.learned.frames > 0, report.learned.won());
        assert_eq!(report.learned.actions.len(), report.learned.frames as usize);
        assert_eq!(report.learned.seed, 1);
        assert_eq!(report.learned.level_hash, level.content_hash_string());
        assert!(!report.learned.note.is_empty());
        assert_eq!(
            report.plan.as_ref().map(|p| p.level_hash.clone()),
            Some(level.content_hash_string())
        );
    }
}
