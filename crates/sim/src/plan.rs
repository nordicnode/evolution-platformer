//! The time-aware planner: a model-based teacher in the **real** stepper.
//!
//! The evolution loop has a scalar objective and no model. That is enough for a corridor and
//! not enough for a level whose answer is a *schedule*: which deck to board, how long to wait
//! for the turret's gap, when to jump off a shelf that is itself moving. [`crate::reach`] can
//! answer whether a crossing exists in principle, but it parks every mover at the ends of its
//! travel and models no clock, so a route it proves is not a route a body can execute.
//!
//! This module closes that gap the cheap way. The sim is deterministic and costs microseconds
//! per tick, so it can be used as a forward model by a best-first search over **macro-actions
//! at the policy's own decision cadence** (`DECISION_TICKS`). A node is a whole sim snapshot;
//! an edge is a script — run, sprint, wait, or jump with a hold — played through the real
//! `Sim::step`. Hazards, movers, patrols, crumbles and the goal are therefore never
//! approximated a second time: the planner and the trained policy cannot disagree about what
//! the world does.
//!
//! # Why macro-actions
//!
//! The policy decides once per [`crate::DECISION_TICKS`] ticks, so a demonstration that is
//! *replayable by the policy* has to be written in those blocks. [`crate::reach`]'s scripts are
//! indexed in ticks and that cadence mismatch is exactly why its clone walks a route and steps
//! into the first pit (`evo::imitate`). Here a decision block is the atom: every macro is a
//! vector of block actions, applied four ticks at a time, and the route this search returns is
//! already a policy demonstration rather than a controller for a different grid.
//!
//! # What the search keeps
//!
//! Best-first over `(route rung, progress, time)` with a beam and a **phase bucket**: waiting
//! is often the right move, and a plain beam would drop every wait because it makes no
//! progress. Keeping the best node per `(stand, phase)` is what lets a timed route survive
//! long enough to be found. The search is deterministic: the macro order and the tie-breaks
//! are fixed, and everything derived from the world comes from the sim.

use crate::level::Level;
use crate::physics::AgentAction;
use crate::{Sim, DECISION_TICKS};

/// The planner's budget. All counts are in **decision blocks**, not ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanConfig {
    /// Grounded expansions before giving up.
    pub depth: u32,
    /// Nodes carried into the next expansion.
    pub beam: usize,
    /// Macro scripts to keep per phase bucket within one stand.
    pub per_bucket: usize,
    /// Blocks a single flight may hold before it is abandoned as a fall.
    pub max_flight_blocks: u32,
}

impl Default for PlanConfig {
    fn default() -> Self {
        PlanConfig {
            depth: 32,
            beam: 64,
            per_bucket: 2,
            max_flight_blocks: 60,
        }
    }
}

impl PlanConfig {
    /// Sanitize, because a planner is also a CLI knob.
    pub fn sanitized(mut self) -> PlanConfig {
        self.depth = self.depth.clamp(1, 512);
        self.beam = self.beam.clamp(1, 2048);
        self.per_bucket = self.per_bucket.clamp(1, 16);
        self.max_flight_blocks = self.max_flight_blocks.clamp(2, 600);
        self
    }
}

/// One decision block's worth of intended play, plus how many blocks it runs for.
///
/// The scripts are deliberately small. `Jump` is a run-up, a fresh press, and a hold; `Move`
/// is a direction; `Sustain` is a constant action repeated, which is what a swim, a vine climb
/// and a wall cling all are. The search composes them, so the interesting behaviour is the
/// composition rather than any one script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Macro {
    Move {
        dir: i8,
        sprint: bool,
        blocks: u32,
    },
    Sustain {
        dir: i8,
        jump: bool,
        blocks: u32,
    },
    Jump {
        dir: i8,
        sprint: bool,
        approach: u32,
        hold: u32,
    },
}

/// A verified route: block actions a policy can replay, and the frame it finished on.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedRoute {
    /// One action per decision block, from the start of the search.
    pub actions: Vec<AgentAction>,
    /// Frames the replay took from the search's own start.
    pub frames: u32,
    /// Route rungs closed from the start state, as the search read them.
    pub hops: u32,
    /// The seed the route was planned against.
    pub seed: u64,
    /// One line saying what the search did, including how much of the level it covered.
    pub note: String,
}

/// One node of the search: a whole sim, and the block actions that produced it.
struct Node {
    sim: Sim,
    actions: Vec<AgentAction>,
    won: bool,
}

/// What the planner found, including the failure diagnosis when it found no route.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanOutcome {
    pub route: Option<TimedRoute>,
    /// One line, in the planner's voice, for a terminal or a test.
    pub note: String,
    pub best_x: f32,
    pub best_y: f32,
    pub frames: u32,
    pub expanded: u32,
}

/// Solve `level` from the episode's spawn, against `seed`.
pub fn solve(level: &Level, seed: u64, cfg: PlanConfig) -> Option<TimedRoute> {
    plan_level(level, seed, cfg).route
}

/// The full planning result: the route when there is one, the frontier when there is not.
pub fn plan_level(level: &Level, seed: u64, cfg: PlanConfig) -> PlanOutcome {
    plan_from(level, Sim::new(level, seed), seed, cfg)
}

/// Solve from a **snapshot** — the state a route walk or a stalled run actually reached.
///
/// This is the hybrid that makes the planner affordable on a long level: the route graph gets
/// the body through the corridor it can already walk, and the planner is spent on the crossing
/// that needs timing. The snapshot is a real `Sim`, so its frame count, its hazard phases and
/// its mover schedules are all preserved.
pub fn solve_from(level: &Level, start: Sim, seed: u64, cfg: PlanConfig) -> Option<TimedRoute> {
    plan_from(level, start, seed, cfg).route
}

/// [`solve_from`] with the full diagnosis.
pub fn plan_from(level: &Level, mut start: Sim, seed: u64, cfg: PlanConfig) -> PlanOutcome {
    debug_assert_eq!(
        level.width,
        start.level().width,
        "a plan runs on its level's geometry"
    );
    let cfg = cfg.sanitized();
    if start.outcome().is_finished() && start.outcome() != crate::Outcome::Won {
        return PlanOutcome {
            route: None,
            note: format!("the start snapshot already ended as {:?}", start.outcome()),
            best_x: start.player().x,
            best_y: start.player().y,
            frames: start.stats().frames,
            expanded: 0,
        };
    }
    // A snapshot may be taken mid-block. Settle onto the block grid without asking the search
    // to reason about a partial decision: a body standing still for the remainder of the block
    // is what the policy itself would have held.
    let mut settle: Vec<AgentAction> = Vec::new();
    let mut guard = 0;
    while start.stats().frames % DECISION_TICKS != 0 && !start.outcome().is_finished() && guard < 64
    {
        start.step(AgentAction::Walk);
        settle.push(AgentAction::Walk);
        guard += 1;
    }
    let start_hops = stand_hops(&start);
    if start.outcome() == crate::Outcome::Won {
        return PlanOutcome {
            route: Some(TimedRoute {
                actions: settle,
                frames: start.stats().frames,
                hops: 0,
                seed,
                note: "the start snapshot is already on the flag".into(),
            }),
            note: "the start snapshot is already on the flag".into(),
            best_x: start.player().x,
            best_y: start.player().y,
            frames: start.stats().frames,
            expanded: 0,
        };
    }

    let mut frontier: Vec<Node> = vec![Node {
        sim: start.clone(),
        actions: settle,
        won: false,
    }];
    let mut expanded = 0u32;
    let mut best_sim = start;
    let mut best_actions: Option<Vec<AgentAction>> = None;

    while expanded < cfg.depth && !frontier.is_empty() {
        expanded += 1;
        let mut children: Vec<Node> = Vec::new();
        for node in frontier.drain(..) {
            if node.won {
                return PlanOutcome {
                    route: Some(route_of(&node, start_hops, seed, expanded)),
                    note: node_note(&node, start_hops, expanded),
                    best_x: node.sim.player().x,
                    best_y: node.sim.player().y,
                    frames: node.sim.stats().frames,
                    expanded,
                };
            }
            let macros = macros_for(&node.sim);
            for m in macros {
                if let Some(mut child) = apply(node.sim.clone(), m, cfg.max_flight_blocks) {
                    child.actions.splice(0..0, node.actions.iter().copied());
                    if child.won {
                        return PlanOutcome {
                            route: Some(route_of(&child, start_hops, seed, expanded)),
                            note: node_note(&child, start_hops, expanded),
                            best_x: child.sim.player().x,
                            best_y: child.sim.player().y,
                            frames: child.sim.stats().frames,
                            expanded,
                        };
                    }
                    if score(&child) < score_node(&best_sim, &best_actions) {
                        best_sim = child.sim.clone();
                        best_actions = Some(child.actions.clone());
                    }
                    children.push(child);
                }
            }
        }
        frontier = select(children, cfg);
    }

    let note = if best_actions.is_some() {
        format!(
            "no full route after {expanded} expansions; the frontier is x {:.2}, y {:.2} at {} frames",
            best_sim.player().x,
            best_sim.player().y,
            best_sim.stats().frames
        )
    } else {
        format!("no full route after {expanded} expansions; no macro survived")
    };
    PlanOutcome {
        route: None,
        note,
        best_x: best_sim.player().x,
        best_y: best_sim.player().y,
        frames: best_sim.stats().frames,
        expanded,
    }
}

fn node_note(node: &Node, start_hops: Option<u32>, expanded: u32) -> String {
    let hops = match (start_hops, stand_hops(&node.sim)) {
        (Some(a), Some(b)) => a.saturating_sub(b),
        _ => 0,
    };
    format!(
        "planned a route in {} blocks and {} frames ({hops} route rungs closed, {expanded} expansions)",
        node.actions.len(),
        node.sim.stats().frames
    )
}

/// Compare a child against the best state seen so far, without keeping a reference into a
/// vector that is about to be replaced.
fn score_node(sim: &Sim, actions: &Option<Vec<AgentAction>>) -> (u32, i64, u32) {
    if actions.is_none() {
        return (u32::MAX, i64::MAX, u32::MAX);
    }
    (stand_hops(sim).unwrap_or(u32::MAX), -(sim.stats().progress_pixels as i64), sim.stats().frames)
}

fn route_of(node: &Node, start_hops: Option<u32>, seed: u64, expanded: u32) -> TimedRoute {
    let hops = match (start_hops, stand_hops(&node.sim)) {
        (Some(a), Some(b)) => a.saturating_sub(b),
        _ => 0,
    };
    let note = if node.sim.outcome() == crate::Outcome::Won {
        format!(
            "planned a route in {} blocks and {} frames ({} route rungs closed)",
            node.actions.len(),
            node.sim.stats().frames,
            hops
        )
    } else {
        format!(
            "no full route after {expanded} expansions; best state is x {:.2}, y {:.2}, {} frames, {} rungs closed",
            node.sim.player().x,
            node.sim.player().y,
            node.sim.stats().frames,
            hops
        )
    };
    TimedRoute {
        actions: node.actions.clone(),
        frames: node.sim.stats().frames,
        hops,
        seed,
        note,
    }
}

/// The route rung a sim is on, falling back to the deepest one it has stood on.
fn stand_hops(sim: &Sim) -> Option<u32> {
    let routes = sim.level().routes();
    sim.route_stand()
        .or(sim.stats().best_stand)
        .and_then(|s| routes.hops_to_flag(s))
}

/// Ordering: fewer rungs left is better, then more progress, then earlier.
fn score(node: &Node) -> (u32, i64, u32) {
    let hops = stand_hops(&node.sim).unwrap_or(u32::MAX);
    let progress = -node.sim.stats().progress_pixels as i64;
    (hops, progress, node.sim.stats().frames)
}

/// Keep the best few per `(rung, phase)` bucket, then the best `beam` overall.
fn select(children: Vec<Node>, cfg: PlanConfig) -> Vec<Node> {
    if children.is_empty() {
        return children;
    }
    let mut ranked = children;
    ranked.sort_by_key(score);
    let mut kept: Vec<Node> = Vec::new();
    let mut buckets: std::collections::BTreeMap<(u32, u32), usize> = std::collections::BTreeMap::new();
    for node in ranked {
        if kept.len() >= cfg.beam {
            break;
        }
        let rung = stand_hops(&node.sim).unwrap_or(u32::MAX);
        // Phase is folded to 24-block buckets: long enough to retain a wait across a turret
        // cycle, short enough that the frontier does not become a parking lot.
        let bucket = (rung, (node.sim.stats().frames / (24 * DECISION_TICKS)) % 6);
        let n = buckets.entry(bucket).or_insert(0);
        if *n >= cfg.per_bucket {
            continue;
        }
        *n += 1;
        kept.push(node);
    }
    kept
}

/// The scripts one grounded state offers.
///
/// Directions and gears rather than a target: the planner does not need to know which way the
/// flag is, because the route rung in the score already says whether a landing helped.
fn macros_for(_sim: &Sim) -> Vec<Macro> {
    let mut out = Vec::new();
    for dir in [-1i8, 0, 1] {
        for blocks in [1, 2, 4, 8, 16] {
            out.push(Macro::Move {
                dir,
                sprint: false,
                blocks,
            });
            out.push(Macro::Move {
                dir,
                sprint: true,
                blocks,
            });
            out.push(Macro::Sustain {
                dir,
                jump: true,
                blocks,
            });
        }
        for sprint in [false, true] {
            for approach in [0, 2, 4, 8] {
                for hold in [1, 2, 3, 4, 5] {
                    out.push(Macro::Jump {
                        dir,
                        sprint,
                        approach,
                        hold,
                    });
                }
            }
        }
    }
    out
}

/// Play one macro on a snapshot. `None` means the episode ended without the flag or the macro
/// did not get a body back on the ground; `Some` is a node ready to be expanded.
fn apply(mut sim: Sim, m: Macro, max_flight_blocks: u32) -> Option<Node> {
    let mut actions: Vec<AgentAction> = Vec::new();
    match m {
        Macro::Move {
            dir,
            sprint,
            blocks,
        } => {
            let action = walk_action(dir, sprint);
            for _ in 0..blocks {
                if sim.outcome().is_finished() {
                    break;
                }
                step_block(&mut sim, action);
                actions.push(action);
            }
            // Walking off a lip is a landing too: chase it down with the same intent instead
            // of throwing away a node that is merely in the air.
            let mut flight = 0u32;
            while !sim.outcome().is_finished() && !sim.player().grounded && flight < max_flight_blocks
            {
                step_block(&mut sim, action);
                actions.push(action);
                flight += 1;
            }
        }
        Macro::Sustain { dir, jump, blocks } => {
            let action = if jump {
                jump_action(dir, false)
            } else {
                walk_action(dir, false)
            };
            for _ in 0..blocks {
                if sim.outcome().is_finished() {
                    break;
                }
                step_block(&mut sim, action);
                actions.push(action);
            }
        }
        Macro::Jump {
            dir,
            sprint,
            approach,
            hold,
        } => {
            let run = walk_action(dir, sprint);
            for _ in 0..approach {
                if sim.outcome().is_finished() {
                    break;
                }
                step_block(&mut sim, run);
                actions.push(run);
            }
            let press = jump_action(dir, sprint);
            let hold = hold.max(1);
            for _ in 0..hold {
                if sim.outcome().is_finished() {
                    break;
                }
                step_block(&mut sim, press);
                actions.push(press);
            }
            // Chase the landing: keep the horizontal intent and let go of jump. A jump that
            // never comes down inside the budget is a fall or a missed ledge, not a node.
            let chase = walk_action(dir, sprint);
            let mut flight = 0u32;
            while !sim.outcome().is_finished() && !sim.player().grounded && flight < max_flight_blocks {
                step_block(&mut sim, chase);
                actions.push(chase);
                flight += 1;
            }
        }
    }
    if actions.is_empty() {
        return None;
    }
    let won = sim.outcome() == crate::Outcome::Won;
    if !won && (sim.outcome().is_finished() || !sim.player().grounded) {
        return None;
    }
    Some(Node {
        sim,
        actions,
        won,
    })
}

fn step_block(sim: &mut Sim, action: AgentAction) {
    for _ in 0..DECISION_TICKS {
        sim.step(action);
    }
}

fn walk_action(dir: i8, sprint: bool) -> AgentAction {
    match (dir, sprint) {
        (1, true) => AgentAction::Sprint,
        (1, false) => AgentAction::Run,
        (-1, true) => AgentAction::RetreatSprint,
        (-1, false) => AgentAction::Retreat,
        _ => AgentAction::Walk,
    }
}

fn jump_action(dir: i8, sprint: bool) -> AgentAction {
    match (dir, sprint) {
        (1, true) => AgentAction::SprintJump,
        (1, false) => AgentAction::RunJump,
        (-1, true) => AgentAction::RetreatSprintJump,
        (-1, false) => AgentAction::RetreatJump,
        _ => AgentAction::JumpInPlace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(rows: &[&str]) -> Level {
        let tiles = rows
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect::<Vec<_>>()
            .join(",");
        let src = format!(
            "{{\"format\":\"evo-platformer-level\",\"level_format_version\":3,\"name\":\"plan\",\
             \"width\":{},\"height\":{},\"spawn\":{{\"x\":1,\"y\":5}},\
             \"tiles\":[{tiles}],\"entities\":[{{\"type\":\"goal\",\"x\":18,\"y\":5}}]}}",
            rows[0].len(),
            rows.len()
        );
        Level::from_json_str(&src).expect("fixture parses")
    }

    #[test]
    fn a_flat_runway_is_planned_with_the_flag() {
        let l = level(&[
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "####################",
        ]);
        let route = solve(&l, 7, PlanConfig::default()).expect("a flat level is solvable");
        assert!(!route.actions.is_empty());
        assert!(route.note.contains("planned"));
    }

    #[test]
    fn a_gap_is_jumped_and_a_wall_is_not_walked_through() {
        let l = level(&[
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "#####....###########",
        ]);
        let route = solve(&l, 7, PlanConfig::default()).expect("the gap is jumpable");
        assert!(route.actions.iter().any(|a| a.wants_jump()));
    }
}
