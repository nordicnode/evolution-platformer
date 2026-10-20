//! **The route plan (Phase 1): one certified route, cut into the legs the telemetry counts.**
//!
//! Two artefacts already answer half of "is there a way through this level": [`crate::reach`]
//! proves the flag is reachable in the jump graph, and [`crate::plan`] searches the real stepping
//! sim for a sequence of decision blocks that wins. Neither is a *plan* in the sense a learner
//! needs: the oracle's route is indexed in ticks and cannot be replayed by a policy that decides
//! once per block, and the planner's answer is one flat list of actions with no account of where
//! the route's legs begin and end.
//!
//! This module joins them. [`compile`] certifies a level twice — the oracle says the flag can be
//! reached, and the planner's route is **replayed** through the same `Sim::step` the trainer uses
//! and must win — and then cuts the certified action list into [`Leg`]s. A leg is exactly what
//! [`crate::EpisodeStats::route_hops_closed`] counts: one canonical route crossing, from the
//! spawn stand (or the last crossing) to the next one. The telemetry's `best_end_leg` and this
//! plan's leg index are therefore the same number, and a run's deaths can be read against the
//! plan's own margins without a translation table. Checkpoints are **not** legs: a checkpoint is
//! a reward zone, a leg is a geodesic crossing, and the two disagree on every level that has a
//! checkpoint off the route.
//!
//! # Margins
//!
//! A leg's margin is the **time it leaves unspent**: the frames still in the episode's budget when
//! the leg completes (`budget_frames - end_frame`). That is the slack a learner has before the
//! level times out, and it is the one margin the planner can actually price — the search works in
//! blocks and frames, not in clearance to a hazard, and inventing a geometric margin the planner
//! never measured would be a number with no witness. Margins are therefore non-increasing along a
//! plan, and the last leg's margin is the plan's own win margin. A margin of zero is a leg that
//! cannot afford one wasted block.
//!
//! # The tail
//!
//! The flag is not always a stand: a crossing can touch it mid-air, and a level can be won
//! without closing every canonical crossing on the graph. Blocks after the last counted crossing
//! therefore belong to the **last leg** rather than forming a leg of their own — there is no
//! crossing in them, so they are the end of the leg that just closed. [`RoutePlan::covered`] says
//! how many crossings the plan actually closed; [`leg_count`] says how many the level's route
//! has. Equal is the normal case; fewer means the route reaches the flag by a shortcut, which is
//! reported rather than hidden.
//!
//! # What is refused
//!
//! [`compile`] never returns a plan that has not won a replay. The three failures are kept apart
//! because they mean different things to a caller: [`CompileError::Unbeatable`] (the oracle says
//! no), [`CompileError::Unroutable`] (the oracle says yes, this planner budget found no line —
//! the case a warm start must report instead of silently cloning a worse route), and
//! [`CompileError::ReplayFailed`] (the planner returned a route that does not survive the sim's
//! own replay, which is a disagreement worth surfacing, not a level to train on).

use serde::{Deserialize, Serialize};

use crate::level::Level;
use crate::physics::AgentAction;
use crate::plan::{self, PlanConfig};
use crate::{reach, Sim, DECISION_TICKS};

/// The artifact's own shape version. Bump when a field's meaning changes, not when a level does:
/// a plan is a derived artifact of one level and one planner configuration.
pub const ROUTE_PLAN_VERSION: u32 = 1;

/// One span of the route: the blocks that carry the body from the previous crossing (or the
/// spawn) to the **next** crossing — or, on a route that reaches the flag before closing every
/// rung, from the last crossing to the flag.
///
/// The span is the unit the telemetry reports: its index is `best_end_leg`, because a body that
/// has closed `k` crossings is in leg `k`. That is why the count is the level's rung count rather
/// than the number of crossings the plan actually closed, and why a block that closes two
/// crossings leaves a zero-block leg between them ([`Leg::blocks`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    /// Zero-based, and the same index the telemetry reports as `best_end_leg`.
    pub index: u32,
    /// First decision block of the leg, inclusive.
    pub first_block: usize,
    /// One past the last decision block of the leg, exclusive.
    pub end_block: usize,
    /// Crossings this leg closes: `1` for every leg except a finish span on a route that reaches
    /// the flag before closing the level's last rung, which closes none.
    pub hops: u32,
    /// Frames this leg took, tail blocks included on the last leg.
    pub frames: u32,
    /// The frame the leg completes on, from the spawn.
    pub end_frame: u32,
    /// Frames still in the episode's budget at `end_frame`. See the module docs on margins.
    pub margin_frames: u32,
}

impl Leg {
    /// Decision blocks in the leg. **Can be zero**: when one block lands on a stand two rungs
    /// closer to the flag it closes two crossings, and the second leg is the instant after the
    /// first. Reporting that rather than merging the two is what keeps a leg's index equal to the
    /// telemetry's crossing count.
    pub fn blocks(&self) -> usize {
        self.end_block - self.first_block
    }
}

/// A certified route, decomposed into legs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutePlan {
    pub version: u32,
    /// The level's display name, so a plan file is self-describing.
    pub level: String,
    /// The level's content hash, as `route_cache` writes it: two plans with the same hash are plans
    /// for the same terrain.
    pub level_hash: String,
    /// The episode seed the plan was searched and replayed against.
    pub seed: u64,
    /// The action of every decision block, from the spawn. Replay it at `DECISION_TICKS` ticks per
    /// action and it wins.
    pub actions: Vec<AgentAction>,
    /// Frames the replay took to reach the flag.
    pub frames: u32,
    /// The level's frame budget (`sim::frame_budget`), so a margin is readable without the level.
    pub budget_frames: u32,
    /// Crossings the level's route has from its spawn stand ([`leg_count`]).
    pub hops: u32,
    /// Crossings this plan closed. Equal to [`RoutePlan::hops`] unless the route ends on the flag
    /// without closing every canonical crossing.
    pub covered: u32,
    /// One entry per leg: `hops` of them (the level's rung count), contiguous and covering every
    /// block. A route that ends early has a last leg with `hops == 0`: the finish span.
    pub legs: Vec<Leg>,
    /// The planner's own account of the search, carried so a plan says how it was found.
    pub note: String,
}

impl RoutePlan {
    /// The last leg's margin: the plan's win margin, in frames.
    pub fn margin_frames(&self) -> u32 {
        self.legs
            .last()
            .map(|leg| leg.margin_frames)
            .unwrap_or_else(|| self.budget_frames.saturating_sub(self.frames))
    }

    /// The tightest leg's margin, in frames.
    pub fn tightest_margin(&self) -> u32 {
        self.legs
            .iter()
            .map(|leg| leg.margin_frames)
            .min()
            .unwrap_or_else(|| self.margin_frames())
    }

    /// The certified action list as the planner's own artifact, so a caller that clones a plan
    /// ([`crate::plan::TimedRoute`] consumers) does not have to reassemble it.
    pub fn timed_route(&self) -> crate::plan::TimedRoute {
        crate::plan::TimedRoute {
            actions: self.actions.clone(),
            frames: self.frames,
            hops: self.covered,
            seed: self.seed,
            note: self.note.clone(),
        }
    }

    /// One line a terminal can print: what was planned, how long it took, how it was cut.
    pub fn line(&self) -> String {
        format!(
            "{} legs, {} of {} rungs closed, {} blocks, {} ({} frames of slack; tightest leg {})",
            self.legs.len(),
            self.covered,
            self.hops,
            self.actions.len(),
            crate::format_time(self.frames),
            self.margin_frames(),
            self.tightest_margin()
        )
    }
}

/// Why a level has no certified plan. The three cases are deliberately distinct: a caller deciding
/// whether to warm a population must not treat "the oracle says no" the same as "this budget found
/// no line".
#[derive(Clone, Debug, PartialEq)]
pub enum CompileError {
    /// The reachability oracle says the flag cannot be reached.
    Unbeatable { why: String },
    /// The oracle certifies the level, but this planner budget found no route. Reported, never
    /// papered over with a different (tick-indexed) route.
    Unroutable {
        note: String,
        best_x: f32,
        best_y: f32,
        expanded: u32,
    },
    /// The planner returned a route that does not win when replayed through the sim.
    ReplayFailed { note: String },
}

impl CompileError {
    /// The oracle certifies the level beatable, even though no plan was produced. The distinction
    /// a warm start needs: this is a planner budget's failure, not a property of the level.
    pub fn beatable(&self) -> bool {
        matches!(self, CompileError::Unroutable { .. })
    }

    /// One line, in the planner's or the oracle's own words.
    pub fn line(&self) -> String {
        match self {
            CompileError::Unbeatable { why } => format!("not beatable: {why}"),
            CompileError::Unroutable {
                note,
                best_x,
                best_y,
                expanded,
            } => format!(
                "beatable in principle, but no timed route after {expanded} expansions \
                 (frontier x {best_x:.2}, y {best_y:.2}): {note}"
            ),
            CompileError::ReplayFailed { note } => {
                format!("the planner's route did not survive its replay: {note}")
            }
        }
    }
}

/// Crossings on the level's route from its spawn stand, or `0` when there is none.
///
/// **The one definition of a leg.** The telemetry's census, this module's plan and the learner's
/// death column all read this number; a checkpoint count would be a different and disagreeing
/// quantity.
pub fn leg_count(level: &Level) -> u32 {
    crate::spawn_route_stand(level)
        .and_then(|stand| level.routes().hops_to_flag(stand))
        .unwrap_or(0)
}

/// Certify and compile a plan: oracle, then planner, then a replay that must win.
pub fn compile(level: &Level, seed: u64, cfg: PlanConfig) -> Result<RoutePlan, CompileError> {
    let report = reach::report(level);
    if !report.goal_reached {
        return Err(CompileError::Unbeatable {
            why: report
                .diagnosis
                .unwrap_or_else(|| "the flag cannot be reached".into()),
        });
    }
    let outcome = plan::plan_level(level, seed, cfg);
    let Some(route) = outcome.route else {
        return Err(CompileError::Unroutable {
            note: outcome.note,
            best_x: outcome.best_x,
            best_y: outcome.best_y,
            expanded: outcome.expanded,
        });
    };

    let budget = crate::frame_budget(level);
    let mut sim = Sim::new(level, seed);
    let repeat = DECISION_TICKS.max(1);
    // One mark per **crossing**, at the block that completed it. A block can complete more than
    // one (a landing two rungs closer), so this pushes once per crossing rather than once per
    // block, and the plan's leg indices then match the telemetry's crossing count exactly.
    let mut marks: Vec<(usize, u32)> = Vec::new(); // (block that completed it, frame at completion)
    let mut hops = 0u32;
    for (block, action) in route.actions.iter().enumerate() {
        for _ in 0..repeat {
            if sim.outcome().is_finished() {
                break;
            }
            sim.step(*action);
        }
        let closed = sim.stats().route_hops_closed;
        while hops < closed {
            hops += 1;
            marks.push((block + 1, sim.stats().frames));
        }
        if sim.outcome().is_finished() {
            break;
        }
    }
    if sim.outcome() != crate::Outcome::Won {
        return Err(CompileError::ReplayFailed {
            note: format!(
                "the plan ended as {:?} at ({:.2}, {:.2}) after {} frames, not on the flag",
                sim.outcome(),
                sim.player().x,
                sim.player().y,
                sim.stats().frames
            ),
        });
    }
    let frames = sim.stats().frames;

    let legs = cut(
        &marks,
        route.actions.len(),
        frames,
        budget,
        leg_count(level),
    );

    Ok(RoutePlan {
        version: ROUTE_PLAN_VERSION,
        level: level.name.clone(),
        level_hash: level.content_hash_string(),
        seed,
        actions: route.actions,
        frames,
        budget_frames: budget,
        hops: leg_count(level),
        covered: marks.len() as u32,
        legs,
        note: route.note,
    })
}

/// Cut a certified replay into legs: one per span, contiguous in blocks, with the tail after the
/// last crossing folded into the last leg.
///
/// `marks` is `(block that completed the crossing, frame at completion)`, one entry per crossing
/// and in order. Two entries can share a block — a landing two rungs closer closes two — and then
/// the later leg has no blocks of its own: the crossing happened, it simply cost nothing extra.
///
/// `hops` is the level's rung count. When the route closed fewer than that (it touched the flag
/// without taking the last canonical crossing), the tail becomes its own **finish leg** instead of
/// being folded into the last crossing: the leg indices then still reach every crossing count a
/// policy could die on, which is the whole point of counting spans rather than crossings.
fn cut(
    marks: &[(usize, u32)],
    action_count: usize,
    total_frames: u32,
    budget: u32,
    hops: u32,
) -> Vec<Leg> {
    let mut legs: Vec<Leg> = Vec::with_capacity(hops.max(1) as usize);
    let mut first_block = 0usize;
    let mut previous_frame = 0u32;
    for (end_block, end_frame) in marks {
        legs.push(Leg {
            index: legs.len() as u32,
            first_block,
            end_block: *end_block,
            hops: 1,
            frames: end_frame.saturating_sub(previous_frame),
            end_frame: *end_frame,
            margin_frames: budget.saturating_sub(*end_frame),
        });
        first_block = *end_block;
        previous_frame = *end_frame;
    }
    if legs.len() < hops as usize {
        // The route reached the flag before closing the level's last canonical crossing. The
        // remaining span is a leg with no crossing of its own; a body that had closed the
        // crossings behind it is in exactly this leg when it dies short of the flag.
        legs.push(Leg {
            index: legs.len() as u32,
            first_block,
            end_block: action_count,
            hops: 0,
            frames: total_frames.saturating_sub(previous_frame),
            end_frame: total_frames,
            margin_frames: budget.saturating_sub(total_frames),
        });
    } else if let Some(last) = legs.last_mut() {
        // The tail after the last crossing (the walk onto the flag) belongs to the last leg: the
        // route closed every rung, so there is no further leg for it to be.
        last.end_block = action_count;
        last.frames += total_frames.saturating_sub(last.end_frame);
        last.end_frame = total_frames;
        last.margin_frames = budget.saturating_sub(total_frames);
    }
    legs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A generated practice level is the plan's own certification case: the suite exists because
    /// both authorities say yes, and this pins that the compiler reads them the same way.
    #[test]
    fn a_practice_level_compiles_into_contiguous_legs_that_cover_every_block() {
        let (level, certification) = crate::practice::generate(16);
        assert!(
            certification.certified(),
            "the fixture is a certified level"
        );
        let plan = compile(
            &level,
            1,
            PlanConfig {
                depth: 48,
                ..PlanConfig::default()
            },
        )
        .expect("a certified plan");
        assert_eq!(plan.version, ROUTE_PLAN_VERSION);
        assert_eq!(plan.level, level.name);
        assert!(!plan.actions.is_empty());
        assert!(!plan.legs.is_empty(), "a route with crossings has legs");
        assert_eq!(
            plan.legs.first().unwrap().first_block,
            0,
            "the first leg starts at the spawn"
        );
        assert_eq!(
            plan.legs.last().unwrap().end_block,
            plan.actions.len(),
            "the legs cover every block, tail included"
        );
        let mut previous_end = 0usize;
        let mut previous_margin = u32::MAX;
        for (index, leg) in plan.legs.iter().enumerate() {
            assert_eq!(leg.index, index as u32);
            assert_eq!(leg.first_block, previous_end, "legs are contiguous");
            assert_eq!(leg.hops, 1, "a leg is one crossing");
            assert!(
                leg.index as usize == index,
                "a leg's index is the telemetry's crossing count"
            );
            assert!(leg.hops >= 1);
            assert!(
                leg.margin_frames <= previous_margin,
                "a margin cannot grow along the route"
            );
            previous_end = leg.end_block;
            previous_margin = leg.margin_frames;
        }
        assert_eq!(plan.legs.iter().map(|l| l.hops).sum::<u32>(), plan.covered);
        assert_eq!(
            plan.legs.len() as u32,
            plan.hops,
            "one leg per span: a body that closed k crossings is in leg k"
        );
        assert_eq!(
            plan.legs.iter().map(|l| l.frames).sum::<u32>(),
            plan.frames,
            "the legs' frames add up to the plan's"
        );
        assert_eq!(
            plan.margin_frames(),
            plan.budget_frames - plan.frames,
            "the last leg's margin is the plan's own win margin"
        );
        assert_eq!(
            plan.covered, plan.hops,
            "the practice routes close every rung"
        );
    }

    /// A level with no route at all is refused by the oracle, not planned around.
    #[test]
    fn a_level_without_a_route_is_unbeatable_rather_than_planned() {
        let (mut level, _) = crate::practice::generate(0);
        // Wall the corridor off: the flag is on the far side and nothing can cross.
        for y in 0..level.height {
            level.set_tile(8, y, crate::Tile::Solid);
            level.set_tile(9, y, crate::Tile::Solid);
        }
        match compile(&level, 1, PlanConfig::default()) {
            Err(CompileError::Unbeatable { .. }) => {}
            other => panic!("a walled level must be refused as unbeatable: {other:?}"),
        }
    }

    /// **The case a warm start must report.** A level the oracle certifies, searched with a budget
    /// too small to find its line: the compiler says so, and `beatable()` is true — the planner
    /// ran out, the level did not.
    #[test]
    fn an_oracle_certified_level_that_the_budget_cannot_route_says_so() {
        let (level, _) = crate::practice::generate(16);
        let outcome = compile(
            &level,
            1,
            PlanConfig {
                depth: 1,
                ..PlanConfig::default()
            },
        );
        match outcome {
            Err(err) => {
                assert!(
                    err.beatable(),
                    "the oracle certifies this level, so the error must say so: {err:?}"
                );
                assert!(
                    err.line().contains("beatable in principle"),
                    "{}",
                    err.line()
                );
            }
            Ok(plan) => panic!("depth 1 routed the level after all: {plan:?}"),
        }
    }

    /// **A block can close two crossings.** A landing two rungs closer to the flag completes two
    /// legs at the same block and frame. The plan reports both, the second with no blocks of its
    /// own, rather than merging them — merging would make a plan's leg index stop matching the
    /// telemetry's `route_hops_closed` on exactly the levels with shortcuts.
    #[test]
    fn a_block_that_closes_two_crossings_yields_two_legs() {
        // Block 10 closes two crossings at frame 60; block 25 closes the third at frame 150.
        let legs = cut(&[(10, 60), (10, 60), (25, 150)], 30, 200, 1000, 3);
        assert_eq!(legs.len(), 3);
        assert_eq!(legs[1].first_block, 10);
        assert_eq!(legs[1].end_block, 10, "the second crossing cost no block");
        assert_eq!(legs[1].blocks(), 0);
        assert_eq!(legs[1].frames, 0, "and no frame");
        assert_eq!(legs[1].end_frame, 60);
        assert_eq!(legs[2].first_block, 10);
        assert_eq!(legs[2].end_block, 30, "the tail belongs to the last leg");
        assert_eq!(
            legs.iter().map(|l| l.index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(legs.iter().map(|l| l.frames).sum::<u32>(), 200);
        assert_eq!(legs.last().unwrap().margin_frames, 800);
        assert_eq!(legs.iter().map(|l| l.hops).sum::<u32>(), 3);
        // Contiguous in blocks: the zero-block leg sits at the boundary rather than breaking it.
        for pair in legs.windows(2) {
            assert_eq!(pair[0].end_block, pair[1].first_block);
        }
    }

    /// **A route can reach the flag before closing every rung.** The finish is then a leg of its
    /// own with no crossing, so the leg indices still reach every crossing count a policy can die
    /// on. This is `l11-the-elements`' shape (11 rungs closed of 12), and the reason the count is
    /// the level's spans rather than the plan's crossings.
    #[test]
    fn a_route_that_ends_early_gets_a_finish_leg() {
        // One crossing at block 10, frame 100; the flag at frame 300, with rung 2 never crossed.
        let legs = cut(&[(10, 100)], 30, 300, 1000, 2);
        assert_eq!(legs.len(), 2, "two spans, one crossing");
        assert_eq!(legs[0].hops, 1);
        assert_eq!(legs[0].end_block, 10);
        assert_eq!(legs[1].hops, 0, "the finish closes no crossing");
        assert_eq!(legs[1].index, 1);
        assert_eq!(legs[1].first_block, 10);
        assert_eq!(legs[1].end_block, 30);
        assert_eq!(legs[1].frames, 200);
        assert_eq!(legs.iter().map(|l| l.frames).sum::<u32>(), 300);
        assert_eq!(legs.last().unwrap().margin_frames, 700);
        assert_eq!(legs.iter().map(|l| l.hops).sum::<u32>(), 1);
    }

    /// The leg count is the telemetry's own number, and a plan's coverage is read against it.
    #[test]
    fn the_leg_count_is_the_route_s_hops_not_its_checkpoints() {
        let (level, _) = crate::practice::generate(6);
        let stand = crate::spawn_route_stand(&level).expect("a practice spawn is on a stand");
        assert_eq!(
            leg_count(&level),
            level.routes().hops_to_flag(stand).unwrap_or(0),
            "the count is the route graph's own geodesic"
        );
        assert!(
            leg_count(&level) >= 1,
            "a gap level has at least one crossing"
        );
    }
}
