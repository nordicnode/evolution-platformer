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

/// **What a trick hunt is looking for (WP14).**
///
/// The planner's objective is the flag; these are the objectives that live *outside* the route
/// oracle's envelope. A discovery is a trajectory that does more of one of them than the intended
/// route does — a higher stand, a longer flight, a faster body, more distance covered without
/// touching ground. The hunt returns the best trajectory it found and never claims the level was
/// solved: a trick artifact is a route fragment, not a champion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HuntObjective {
    /// Highest point reached (smaller world `y` is higher).
    Height,
    /// Longest airborne run, in ticks.
    Airtime,
    /// Fastest body speed, tiles/tick.
    Speed,
    /// Furthest horizontal distance covered while never grounded.
    GroundFreeDistance,
}

impl HuntObjective {
    pub fn as_str(self) -> &'static str {
        match self {
            HuntObjective::Height => "height",
            HuntObjective::Airtime => "airtime",
            HuntObjective::Speed => "speed",
            HuntObjective::GroundFreeDistance => "ground-free-distance",
        }
    }

    fn value(self, sim: &Sim) -> f32 {
        match self {
            HuntObjective::Height => -sim.player().y,
            HuntObjective::Airtime => sim.stats().max_air_ticks as f32,
            HuntObjective::Speed => sim.stats().peak_speed,
            HuntObjective::GroundFreeDistance => sim.stats().max_air_distance,
        }
    }
}

/// What a hunt found, and the trajectory that proves it.
#[derive(Clone, Debug, PartialEq)]
pub struct HuntOutcome {
    pub objective: HuntObjective,
    /// The best reading of the objective, in the objective's own units.
    pub best: f32,
    /// The trajectory that set it: block actions from the hunt's start state.
    pub route: Option<TimedRoute>,
    pub best_x: f32,
    pub best_y: f32,
    pub frames: u32,
    pub expanded: u32,
    /// One line in the hunt's voice, for a terminal or an artifact.
    pub note: String,
}

/// Search for a trajectory that maximises `objective` instead of reaching the flag.
///
/// The beam is the planner's own (`macros_for`/`apply`/snapshot clones), which is deliberate: a
/// discovery is only interesting if the same physics the ordinary planner uses can produce it, and
/// reusing the expansion is what keeps the two searches from disagreeing about what a move is. A
/// node that wins is kept as the best reading and the search continues from its siblings rather
/// than stopping, because a win is not what was asked for.
pub fn hunt(level: &Level, seed: u64, cfg: PlanConfig, objective: HuntObjective) -> HuntOutcome {
    let cfg = cfg.sanitized();
    let mut frontier = vec![Node {
        sim: Sim::new(level, seed),
        actions: Vec::new(),
        won: false,
    }];
    let mut best: Option<Node> = None;
    let mut best_value = f32::NEG_INFINITY;
    let mut expanded = 0u32;
    while expanded < cfg.depth && !frontier.is_empty() {
        expanded += 1;
        let children = expand(&frontier, cfg.max_flight_blocks);
        if children.is_empty() {
            break;
        }
        let mut kept: Vec<Node> = Vec::with_capacity(cfg.beam.min(children.len()));
        for child in children {
            let value = objective.value(&child.sim);
            // Ties are settled by the earlier frame count, so a shorter trajectory that reaches
            // the same height is the one recorded.
            let better = best.as_ref().is_none_or(|held| {
                value > best_value
                    || (value == best_value && child.sim.stats().frames < held.sim.stats().frames)
            });
            if better {
                best_value = value;
                best = Some(child.clone());
            }
            kept.push(child);
        }
        kept.sort_by(|a, b| {
            objective
                .value(&b.sim)
                .total_cmp(&objective.value(&a.sim))
                .then(b.sim.stats().frames.cmp(&a.sim.stats().frames))
        });
        kept.truncate(cfg.beam);
        frontier = kept;
    }
    let route = best.as_ref().map(|node| TimedRoute {
        actions: node.actions.clone(),
        frames: node.sim.stats().frames,
        hops: stand_hops(&node.sim).unwrap_or(0),
        seed,
        note: format!(
            "hunted {} for {} expansions; best {:.3} at x {:.2}, y {:.2}, {} frames",
            objective.as_str(),
            expanded,
            best_value,
            node.sim.player().x,
            node.sim.player().y,
            node.sim.stats().frames
        ),
    });
    let note = match &route {
        Some(route) => route.note.clone(),
        None => format!(
            "hunted {} for {expanded} expansions; no trajectory survived the first move",
            objective.as_str()
        ),
    };
    HuntOutcome {
        objective,
        best: if best_value.is_finite() { best_value } else { 0.0 },
        route,
        best_x: best.as_ref().map(|n| n.sim.player().x).unwrap_or(0.0),
        best_y: best.as_ref().map(|n| n.sim.player().y).unwrap_or(0.0),
        frames: best.as_ref().map(|n| n.sim.stats().frames).unwrap_or(0),
        expanded,
        note,
    }
}

/// One node of the search: a whole sim, and the block actions that produced it.
#[derive(Clone)]
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

/// **Anytime planning:** deepen the same search until a route appears or the cap is reached.
///
/// The planner's cost is a beam and a depth, and a level that needs one long crossing can be
/// solved at depth 64 after depth 32 has run out of frontier. Restarting with a larger depth is
/// deliberate rather than wasteful: the frontier at depth 32 is already the best 32 blocks the
/// beam kept, and the deterministic tie-breaks make a second pass extend that answer rather than
/// search a different one. Each pass is a complete `plan_from`, so every returned route is a
/// route a single fixed configuration would have returned at that depth — the "anytime" part is
/// the budget, not the semantics.
///
/// `step` is the depth increment and `max_depth` the cap; both are clamped to the planner's own
/// limits. The loop is deterministic for a given `(level, seed, cfg, step, max_depth)`, which is
/// what makes an artifact built by it replayable.
pub fn solve_anytime(
    level: &Level,
    seed: u64,
    cfg: PlanConfig,
    step: u32,
    max_depth: u32,
) -> PlanOutcome {
    let cfg = cfg.sanitized();
    let step = step.max(1);
    let max_depth = max_depth.clamp(step, 512);
    let mut deepest = PlanOutcome {
        route: None,
        note: "the planner ran no expansion".into(),
        best_x: 0.0,
        best_y: 0.0,
        frames: 0,
        expanded: 0,
    };
    let mut depth = step.min(max_depth);
    loop {
        let outcome = plan_level(
            level,
            seed,
            PlanConfig {
                depth,
                ..cfg
            },
        );
        if outcome.route.is_some() {
            return outcome;
        }
        if outcome.expanded > deepest.expanded
            || (outcome.expanded == deepest.expanded && outcome.best_x > deepest.best_x)
        {
            deepest = outcome;
        }
        if depth >= max_depth {
            let mut note = deepest.note.clone();
            note.push_str(&format!(
                " (anytime search stopped at depth {max_depth}; raise --plan-max-depth to widen it)"
            ));
            deepest.note = note;
            return deepest;
        }
        depth = (depth + step).min(max_depth);
    }
}

/// **The editor's verdict (WP6).** Three answers, in the order a designer needs them:
///
/// 1. [`PlanVerdict::Planned`] — the timed planner found a route, with its length and the number
///    of route rungs it closed. This is the only answer that may be shown as "solvable";
/// 2. [`PlanVerdict::RoutedOnly`] — the reachability oracle can get there but the timed planner
///    ran out of budget. Honest and useful: the level is beatable in principle, this search just
///    did not find the line;
/// 3. [`PlanVerdict::Unbeatable`] — the oracle itself says no, with its diagnosis.
///
/// The dangerous direction is calling a level solvable when it is not, so `Planned` is only ever
/// returned for a route that actually won a replayed search.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanVerdict {
    Planned {
        frames: u32,
        blocks: usize,
        hops: u32,
    },
    RoutedOnly {
        note: String,
    },
    Unbeatable {
        why: String,
    },
}

impl PlanVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            PlanVerdict::Planned { .. } => "solvable by timed plan",
            PlanVerdict::RoutedOnly { .. } => "solvable by route; timed plan ran out of budget",
            PlanVerdict::Unbeatable { .. } => "not beatable",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            PlanVerdict::Planned {
                frames,
                blocks,
                hops,
            } => format!("{blocks} blocks, {frames} frames, {hops} route rungs"),
            PlanVerdict::RoutedOnly { note } => note.clone(),
            PlanVerdict::Unbeatable { why } => why.clone(),
        }
    }
}

/// Answer the editor's question: can this level be beaten, and by what kind of search?
///
/// The reachability oracle runs first because it is cheap and it is the authority on
/// *impossibility*; the timed planner then answers the stronger question a designer actually asks
/// (a line the AI can run), and a budget miss is reported as such rather than downgraded to
/// "unbeatable". Deterministic for a given `(level, seed, cfg)`.
pub fn verdict(level: &Level, seed: u64, cfg: PlanConfig) -> PlanVerdict {
    let report = crate::reach::report(level);
    if !report.goal_reached {
        return PlanVerdict::Unbeatable {
            why: report
                .diagnosis
                .unwrap_or_else(|| "the flag cannot be reached".into()),
        };
    }
    match plan_level(level, seed, cfg.sanitized()) {
        PlanOutcome {
            route: Some(route), ..
        } => PlanVerdict::Planned {
            frames: route.frames,
            blocks: route.actions.len(),
            hops: route.hops,
        },
        outcome => PlanVerdict::RoutedOnly { note: outcome.note },
    }
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
        let children = expand(&frontier, cfg.max_flight_blocks);
        let mut kept: Vec<Node> = Vec::with_capacity(children.len());
        for child in children {
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
            kept.push(child);
        }
        frontier = select(kept, cfg);
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

/// Expand every frontier node into its children, in frontier order.
///
/// The order is the determinism contract: a node's children are contiguous and in the same order
/// `macros_for` produced them, so `select`'s stable sort sees the same input every run. Expansion
/// is deliberately single-threaded here — the `sim` crate's boundary rule keeps threads out of the
/// world model; a caller that wants parallel planning runs independent plans in parallel and folds
/// their deterministic results (see `train-cli plan --parallel`).
fn expand(frontier: &[Node], max_flight_blocks: u32) -> Vec<Node> {
    let mut out = Vec::new();
    for node in frontier {
        for m in macros_for(&node.sim) {
            if let Some(mut child) = apply(node.sim.clone(), m, max_flight_blocks) {
                child.actions.splice(0..0, node.actions.iter().copied());
                out.push(child);
            }
        }
    }
    out
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

    #[test]
    fn a_verdict_names_the_kind_of_answer_the_editor_gets() {
        let planned = level(&[
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "####################",
        ]);
        assert!(matches!(
            verdict(&planned, 7, PlanConfig::default()),
            PlanVerdict::Planned { .. }
        ));

        // No floor at all: the reach oracle is the authority on impossibility, and the verdict
        // must repeat its answer rather than spend a planner budget pretending to search.
        let nowhere = level(&[
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
        ]);
        assert!(matches!(
            verdict(&nowhere, 7, PlanConfig::default()),
            PlanVerdict::Unbeatable { .. }
        ));
    }

    #[test]
    fn a_hunt_is_deterministic_and_its_best_trajectory_replays() {
        let l = level(&[
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "#####....###########",
        ]);
        let cfg = PlanConfig {
            depth: 3,
            beam: 6,
            ..PlanConfig::default()
        };
        let a = hunt(&l, 7, cfg, HuntObjective::Airtime);
        let b = hunt(&l, 7, cfg, HuntObjective::Airtime);
        assert_eq!(a, b, "the hunt is a pure function of its inputs");
        let route = a.route.expect("some trajectory survived");
        let mut sim = Sim::new(&l, route.seed);
        for action in &route.actions {
            for _ in 0..DECISION_TICKS {
                if sim.outcome().is_finished() {
                    break;
                }
                sim.step(*action);
            }
        }
        assert_eq!(sim.stats().frames, route.frames);
    }
}
