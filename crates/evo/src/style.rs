//! **Style titles (WP15): constrained solutions beside the champion.**
//!
//! Banning the obvious tool is the most reliable trick-invention machine there is, which is why
//! speedrun categories produce technology. This module is the constraint vocabulary and the
//! verifier; it deliberately does not run a search of its own. The caller supplies candidate
//! episodes (a champion, a league's population, a campaign's segments) and a title is awarded only
//! when the candidate **won under the constraint**, judged from the tick-action route it walked.
//!
//! # What is checked, and what is not
//!
//! * **Fastest** — won; lowest frame count.
//! * **Fewest jumps** — won; lowest jump count.
//! * **No sprint** — won with no sprint action anywhere in the route.
//! * **No wall jump** — won with zero wall jumps recorded by the sim.
//! * **Minimalist** — won; fewest action changes (the fewest decisions a policy had to make).
//!
//! **Stomp-only crossings is not implemented, and that is a retirement rather than an omission.**
//! The plan defines it as "crossings of gaps ≥3 tiles must begin with a stomp", which needs a
//! segmentation of the route into crossings. A [`TasRoute`](crate::tas::TasRoute) records actions
//! and a final state, not the take-off and landing stands, so this module cannot verify the
//! constraint without inventing a definition the rest of the codebase does not share. A title
//! nobody can verify is worse than no title: it would be awarded by whichever run printed it.
//!
//! Every check is a pure function of the route and the episode reading, so two front-ends cannot
//! disagree about who holds a title.

use crate::fitness::EpisodeResult;
use crate::tas::TasRoute;
use sim::physics::AgentAction;

/// One constrained championship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Style {
    Fastest,
    FewestJumps,
    NoSprint,
    NoWallJump,
    Minimalist,
}

impl Style {
    pub const ALL: [Style; 5] = [
        Style::Fastest,
        Style::FewestJumps,
        Style::NoSprint,
        Style::NoWallJump,
        Style::Minimalist,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Style::Fastest => "Fastest",
            Style::FewestJumps => "Fewest jumps",
            Style::NoSprint => "No sprint",
            Style::NoWallJump => "No wall jump",
            Style::Minimalist => "Minimalist",
        }
    }

    pub fn constraint(self) -> &'static str {
        match self {
            Style::Fastest => "lowest frames to the flag",
            Style::FewestJumps => "lowest jump count",
            Style::NoSprint => "no sprint action at any tick",
            Style::NoWallJump => "zero wall jumps",
            Style::Minimalist => "fewest action changes",
        }
    }
}

/// One candidate's verdict under one style.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleCheck {
    pub style: Style,
    /// Won, and broke no constraint.
    pub eligible: bool,
    /// The number the title is ranked by (lower is better) when eligible.
    pub score: f32,
    /// Why it failed, when it did.
    pub reason: Option<String>,
}

impl StyleCheck {
    fn refused(style: Style, reason: impl Into<String>) -> StyleCheck {
        StyleCheck {
            style,
            eligible: false,
            score: f32::INFINITY,
            reason: Some(reason.into()),
        }
    }
}

/// A title and the candidate that holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleTitle {
    pub style: Style,
    pub genome_id: u32,
    pub score: f32,
    pub frames: u32,
    /// One line for a shelf card or a report.
    pub line: String,
}

/// Does `route` contain a sprint action?
pub fn uses_sprint(route: &TasRoute) -> bool {
    route.actions.iter().any(|a| {
        matches!(
            a,
            AgentAction::Sprint
                | AgentAction::SprintJump
                | AgentAction::RetreatSprint
                | AgentAction::RetreatSprintJump
        )
    })
}

/// How many times the held action changed. The first action is not a change.
pub fn action_changes(route: &TasRoute) -> u32 {
    route
        .actions
        .windows(2)
        .filter(|pair| pair[0] != pair[1])
        .count() as u32
}

/// Check one episode against one style.
///
/// A route that did not win is never eligible: a constrained *solution* is a solution first.
pub fn check(style: Style, route: &TasRoute, result: &EpisodeResult) -> StyleCheck {
    if !route.won() {
        return StyleCheck::refused(style, "the episode did not win");
    }
    match style {
        Style::Fastest => StyleCheck {
            style,
            eligible: true,
            score: route.frames as f32,
            reason: None,
        },
        Style::FewestJumps => StyleCheck {
            style,
            eligible: true,
            score: result.jumps as f32,
            reason: None,
        },
        Style::NoSprint => {
            if uses_sprint(route) {
                StyleCheck::refused(style, "the route sprinted at least once")
            } else {
                StyleCheck {
                    style,
                    eligible: true,
                    score: route.frames as f32,
                    reason: None,
                }
            }
        }
        Style::NoWallJump => {
            if result.wall_jumps > 0 {
                StyleCheck::refused(
                    style,
                    format!("the route used {} wall jump(s)", result.wall_jumps),
                )
            } else {
                StyleCheck {
                    style,
                    eligible: true,
                    score: route.frames as f32,
                    reason: None,
                }
            }
        }
        Style::Minimalist => StyleCheck {
            style,
            eligible: true,
            score: action_changes(route) as f32,
            reason: None,
        },
    }
}

/// Award every style that has at least one eligible candidate.
///
/// Ties go to the lower genome id so a report is deterministic; the input order does not decide a
/// title.
pub fn award(
    style: Style,
    candidates: &[(u32, &TasRoute, &EpisodeResult)],
) -> Option<StyleTitle> {
    let mut best: Option<(u32, f32, &TasRoute, &EpisodeResult)> = None;
    for (genome_id, route, result) in candidates {
        let check = check(style, route, result);
        if !check.eligible {
            continue;
        }
        let better = best.is_none_or(|(id, score, _, _)| {
            check.score < score || (check.score == score && *genome_id < id)
        });
        if better {
            best = Some((*genome_id, check.score, route, result));
        }
    }
    best.map(|(genome_id, score, route, result)| StyleTitle {
        style,
        genome_id,
        score,
        frames: route.frames,
        line: format!(
            "{}: genome {} — {} ({} frames, {} jumps)",
            style.title(),
            genome_id,
            style.constraint(),
            route.frames,
            result.jumps
        ),
    })
}

/// Every title the candidates can hold, in [`Style::ALL`] order.
pub fn gallery(candidates: &[(u32, &TasRoute, &EpisodeResult)]) -> Vec<StyleTitle> {
    Style::ALL
        .iter()
        .filter_map(|style| award(*style, candidates))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tas::{TasKind, TAS_FORMAT, TAS_VERSION};
    use sim::Outcome;

    fn result(id: u32, jumps: u32, wall_jumps: u32) -> EpisodeResult {
        EpisodeResult {
            genome_id: id,
            fitness: 0.0,
            mean_fitness: 0.0,
            worst_trial: 0.0,
            outcome: Outcome::Won,
            frames: 20,
            progress_pixels: 0.0,
            best_x: 0.0,
            stomps: 0,
            max_stomp_chain: 0,
            wall_jumps,
            springs: 0,
            crumble_burned: 0,
            max_air_ticks: 0,
            max_air_distance: 0.0,
            peak_speed: 0.0,
            jumps,
            checkpoints: 0,
            route_hops_closed: 0,
            best_stand: None,
            suite_wins: 0,
            all_trials_won: true,
            death_pos: None,
            death_cause: None,
            cut_early: false,
            seed: 1,
            trajectory_hash: 0,
        }
    }

    fn route(actions: Vec<AgentAction>) -> TasRoute {
        let frames = actions.len() as u32;
        TasRoute {
            format: TAS_FORMAT.into(),
            version: TAS_VERSION,
            kind: TasKind::Unpolished,
            level_name: "test".into(),
            level_hash: "hash".into(),
            source_genome_id: Some(1),
            seed: 1,
            actions,
            frames,
            final_state_hash: 0,
            note: String::new(),
        }
    }

    #[test]
    fn a_sprinting_route_cannot_hold_the_no_sprint_title() {
        let r = route(vec![AgentAction::Sprint, AgentAction::Sprint]);
        let e = result(1, 0, 0);
        assert!(!check(Style::NoSprint, &r, &e).eligible);
        let walk = route(vec![AgentAction::Walk, AgentAction::Walk]);
        assert!(check(Style::NoSprint, &walk, &e).eligible);
    }

    #[test]
    fn a_wall_jump_reading_disqualifies_the_wall_title() {
        let r = route(vec![AgentAction::Run]);
        assert!(!check(Style::NoWallJump, &r, &result(1, 0, 1)).eligible);
        assert!(check(Style::NoWallJump, &r, &result(1, 0, 0)).eligible);
    }

    #[test]
    fn minimalist_counts_decisions_not_ticks() {
        let r = route(vec![
            AgentAction::Walk,
            AgentAction::Walk,
            AgentAction::JumpInPlace,
            AgentAction::JumpInPlace,
        ]);
        assert_eq!(action_changes(&r), 1);
        assert_eq!(check(Style::Minimalist, &r, &result(1, 0, 0)).score, 1.0);
    }

    #[test]
    fn the_gallery_picks_the_better_candidate_and_reports_a_refusal() {
        let fast = route(vec![AgentAction::Walk; 10]);
        let slow = route(vec![AgentAction::Walk; 20]);
        let fast_result = result(1, 2, 0);
        let slow_result = result(2, 1, 0);
        let candidates = [
            (1, &fast, &fast_result),
            (2, &slow, &slow_result),
        ];
        let titles = gallery(&candidates);
        let fastest = titles
            .iter()
            .find(|t| t.style == Style::Fastest)
            .expect("fastest title");
        assert_eq!(fastest.genome_id, 1);
        let fewest = titles
            .iter()
            .find(|t| t.style == Style::FewestJumps)
            .expect("fewest-jumps title");
        assert_eq!(fewest.genome_id, 2);
    }
}
