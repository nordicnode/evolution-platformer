//! **Solver contract.**
//!
//! A level can be "possible" in several different senses. Keeping those answers in one report
//! prevents a reachability graph, a timed route, and a one-seed champion from being presented as
//! interchangeable proof. The contract is deliberately conservative: a planner route is a
//! guaranteed solver for its recorded phase, while a learned policy must pass held-out phases and
//! the optional action-disturbance battery before it is called robust.

use serde::{Deserialize, Serialize};
use sim::{reach, route_plan, Level};

use crate::genome::Genome;
use crate::robust::{
    evaluate as evaluate_robust, evaluate_adversarial, AdversarialConfig, RobustConfig,
};

pub const CONTRACT_FORMAT: &str = "evo-solver-contract";
pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SolverStatus {
    Unreachable,
    ReachableNoTimedPlan,
    PlannerCertified,
    PolicyCertified,
    RobustPolicyCertified,
}

impl SolverStatus {
    pub fn label(self) -> &'static str {
        match self {
            SolverStatus::Unreachable => "unreachable",
            SolverStatus::ReachableNoTimedPlan => "reachable; no timed plan in budget",
            SolverStatus::PlannerCertified => "planner-certified",
            SolverStatus::PolicyCertified => "learned-policy-certified",
            SolverStatus::RobustPolicyCertified => "robust learned-policy-certified",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CertifyConfig {
    pub planner: sim::plan::PlanConfig,
    pub robust_episodes: u32,
    pub adversarial_episodes: u32,
    pub adversarial_cases: u32,
    pub action_repeat: u32,
}

impl Default for CertifyConfig {
    fn default() -> Self {
        CertifyConfig {
            planner: sim::plan::PlanConfig {
                depth: 64,
                ..sim::plan::PlanConfig::default()
            },
            robust_episodes: 24,
            adversarial_episodes: 8,
            adversarial_cases: 3,
            action_repeat: crate::ACTION_REPEAT,
        }
    }
}

impl CertifyConfig {
    pub fn sanitized(self) -> Self {
        CertifyConfig {
            planner: self.planner.sanitized(),
            robust_episodes: self.robust_episodes.clamp(1, 256),
            adversarial_episodes: self.adversarial_episodes.clamp(1, 64),
            adversarial_cases: self.adversarial_cases.clamp(1, 8),
            action_repeat: self.action_repeat.clamp(1, 32),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanCertificate {
    pub seed: u64,
    pub frames: u32,
    pub blocks: usize,
    pub hops: u32,
    pub tightest_margin: u32,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolicyCertificate {
    pub episodes: u32,
    pub wins: u32,
    pub win_rate: f32,
    pub mean_fitness: f32,
    pub worst_fitness: f32,
    pub worst_best_x: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdversarialCertificate {
    pub cases: u32,
    pub wins: u32,
    pub win_rate: f32,
    pub required_win_rate: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolverContract {
    pub format: String,
    pub version: u32,
    pub level_name: String,
    pub level_hash: String,
    pub seed: u64,
    pub status: SolverStatus,
    pub physics_reachable: bool,
    pub reachable_hops: usize,
    pub reach_diagnosis: Option<String>,
    pub planner: Option<PlanCertificate>,
    pub planner_error: Option<String>,
    pub learned_policy: Option<PolicyCertificate>,
    pub robust_validation: Option<PolicyCertificate>,
    pub adversarial_validation: Option<AdversarialCertificate>,
}

impl SolverContract {
    /// A contract is complete when there is a verified solver of record. A policy is an optional
    /// stronger claim; a planner certificate remains sufficient for levels outside the learned
    /// policy's current envelope.
    pub fn complete(&self) -> bool {
        self.planner.is_some()
            || self
                .robust_validation
                .as_ref()
                .is_some_and(|p| p.win_rate >= 1.0)
    }

    pub fn line(&self) -> String {
        let plan = self
            .planner
            .as_ref()
            .map(|p| {
                format!(
                    "{} blocks, {} frames, margin {}",
                    p.blocks, p.frames, p.tightest_margin
                )
            })
            .unwrap_or_else(|| "none".into());
        let policy = self
            .robust_validation
            .as_ref()
            .map(|p| format!("{}/{} held-out wins", p.wins, p.episodes))
            .unwrap_or_else(|| "not supplied".into());
        format!(
            "{}: physics {}, planner {}, policy {}",
            self.status.label(),
            if self.physics_reachable {
                "reachable"
            } else {
                "unreachable"
            },
            plan,
            policy
        )
    }
}

/// Build the complete contract. `genome` may be omitted for editor-only certification.
pub fn certify(
    level: &Level,
    genome: Option<&Genome>,
    seed: u64,
    cfg: CertifyConfig,
) -> SolverContract {
    let cfg = cfg.sanitized();
    let reach_report = reach::report(level);
    let mut planner = None;
    let mut planner_error = None;
    if reach_report.goal_reached {
        match route_plan::compile(level, seed, cfg.planner) {
            Ok(plan) => {
                planner = Some(PlanCertificate {
                    seed,
                    frames: plan.frames,
                    blocks: plan.actions.len(),
                    hops: plan.covered,
                    tightest_margin: plan.tightest_margin(),
                    note: plan.note,
                });
            }
            Err(error) => planner_error = Some(error.line()),
        }
    }

    let (learned_policy, robust_validation, adversarial_validation) = if let Some(genome) = genome {
        let quick = evaluate_robust(
            level,
            genome,
            seed,
            RobustConfig {
                episodes: 1,
                action_repeat: cfg.action_repeat,
            },
        );
        let robust = evaluate_robust(
            level,
            genome,
            seed,
            RobustConfig {
                episodes: cfg.robust_episodes,
                action_repeat: cfg.action_repeat,
            },
        );
        let adversarial = evaluate_adversarial(
            level,
            genome,
            seed,
            AdversarialConfig {
                episodes: cfg.adversarial_episodes,
                action_repeat: cfg.action_repeat,
                cases_per_episode: cfg.adversarial_cases,
                ..AdversarialConfig::default()
            },
        );
        let summary = |report: &crate::robust::RobustReport| PolicyCertificate {
            episodes: report.episodes,
            wins: report.wins,
            win_rate: report.win_rate(),
            mean_fitness: report.mean_fitness,
            worst_fitness: report.worst_fitness,
            worst_best_x: report.worst_best_x,
        };
        (
            Some(summary(&quick)),
            Some(summary(&robust)),
            Some(AdversarialCertificate {
                cases: adversarial.cases,
                wins: adversarial.wins,
                win_rate: adversarial.win_rate,
                required_win_rate: AdversarialConfig::default().required_win_rate,
            }),
        )
    } else {
        (None, None, None)
    };

    let robust_policy = robust_validation
        .as_ref()
        .is_some_and(|report| report.win_rate >= 1.0)
        && adversarial_validation
            .as_ref()
            .is_some_and(|report| report.win_rate >= report.required_win_rate);
    let status = if !reach_report.goal_reached {
        SolverStatus::Unreachable
    } else if robust_policy {
        SolverStatus::RobustPolicyCertified
    } else if robust_validation
        .as_ref()
        .is_some_and(|report| report.win_rate >= 1.0)
    {
        SolverStatus::PolicyCertified
    } else if planner.is_some() {
        SolverStatus::PlannerCertified
    } else {
        SolverStatus::ReachableNoTimedPlan
    };
    SolverContract {
        format: CONTRACT_FORMAT.into(),
        version: CONTRACT_VERSION,
        level_name: level.name.clone(),
        level_hash: level.content_hash_string(),
        seed,
        status,
        physics_reachable: reach_report.goal_reached,
        reachable_hops: reach_report.reachable_platforms(),
        reach_diagnosis: reach_report.diagnosis,
        planner,
        planner_error,
        learned_policy,
        robust_validation,
        adversarial_validation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_contract_can_certify_a_planner_without_a_policy() {
        let level = Level::from_json_str(include_str!("../../../levels/l1-flat-run.json"))
            .expect("the flat level parses");
        let contract = certify(
            &level,
            None,
            7,
            CertifyConfig {
                planner: sim::plan::PlanConfig {
                    depth: 32,
                    ..sim::plan::PlanConfig::default()
                },
                ..CertifyConfig::default()
            },
        );
        assert!(contract.physics_reachable);
        assert!(contract.planner.is_some());
        assert_eq!(contract.status, SolverStatus::PlannerCertified);
        assert!(contract.complete());
        assert!(contract.learned_policy.is_none());
    }
}
