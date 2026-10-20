//! A model-based controller: **look at the world, simulate, then act**.
//!
//! Everything else in this crate selects a policy from episodes that have already
//! happened. This module does the other kind of thinking. At every decision it holds
//! the real [`Sim`], asks the level's route graph where the next crossing lands, and
//! searches the simulator's own action set forward far enough to see which choice is
//! still alive and closer to that landing. The chosen action is executed, the world
//! advances four ticks, and the process repeats. There is no random choice anywhere in
//! the loop.
//!
//! The search is deliberately small. It is a beam over whole decision blocks — the
//! same cadence the evolved policy uses — so an action sequence it finds can be
//! replayed verbatim. A node is a cloned [`Sim`], which is cheap because the level is
//! shared and the mutable state is a handful of vectors. The heuristic's terms each
//! exist for a measured reason:
//!
//! * **distance to the route's next waypoint** — the same aim the observation's
//!   waypoint sensor uses, not a second guess at where the route goes;
//! * **route hops closed**, because a body can be next to a waypoint and on the wrong
//!   side of a wall;
//! * **progress toward the flag**, which keeps a jump alive while its first blocks are
//!   higher and no closer to the aim point;
//! * **survival**, which is what makes the controller wait. A deck arrives on a
//!   schedule; walking off the ledge now is certain death, while holding still is
//!   neutral, and the beam can see the difference because it runs the real physics
//!   rather than a distance model.
//!
//! The controller is not a replacement for evolution. It is the deliberate half of a
//! hybrid: a reflex net is fast and cheap, and this is what the same body does when
//! the reflex has no answer. [`play`] returns the whole trajectory, so a run can show
//! the thought or use it as a demonstration for the population.

use sim::physics::AgentAction;
use sim::{Level, Outcome, Sim, DECISION_TICKS};

/// One deliberate episode's budget.
///
/// The defaults are sized against the shipped pack: twelve blocks of lookahead is
/// forty-eight ticks, longer than a running jump and the fall after it, and
/// twenty-four nodes wide is enough to keep a wait, both directions and both jump
/// gears alive at once. The cost is bounded by
/// `horizon × beam × actions × DECISION_TICKS` physics steps per decision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThinkConfig {
    /// Decision blocks searched ahead at each step.
    pub horizon: u32,
    /// Nodes kept after each block.
    pub beam: usize,
    /// Weight on the distance to the next route waypoint.
    pub aim_weight: f32,
    /// Weight on route hops still to close. A whole hop is worth this much, so a
    /// candidate that lands on the next rung beats one that is merely closer.
    pub hop_weight: f32,
    /// Hard cap on decisions, so a malformed level cannot loop forever.
    pub max_decisions: u32,
}

impl Default for ThinkConfig {
    fn default() -> Self {
        ThinkConfig {
            // Twelve blocks is forty-eight ticks: long enough to see a running
            // jump's landing and the start of the fall after it. Twenty-four nodes
            // keeps a wait, both directions and both jump gears alive, which is
            // what makes a spike or a patrol survivable rather than a coin flip.
            horizon: 12,
            beam: 24,
            aim_weight: 12.0,
            hop_weight: 80.0,
            max_decisions: 1200,
        }
    }
}

impl ThinkConfig {
    pub fn sanitized(mut self) -> Self {
        self.horizon = self.horizon.clamp(1, 24);
        self.beam = self.beam.clamp(1, 128);
        self.aim_weight = self.aim_weight.max(0.0);
        self.hop_weight = self.hop_weight.max(0.0);
        self.max_decisions = self.max_decisions.clamp(1, 10_000);
        self
    }
}

/// What one deliberate episode did.
#[derive(Clone, Debug, PartialEq)]
pub struct ThinkReport {
    /// One action per decision block, in the order the body took them.
    pub actions: Vec<AgentAction>,
    pub outcome: Outcome,
    pub frames: u32,
    pub best_x: f32,
    /// Route rungs closed by the episode, as the sim counted them.
    pub route_hops_closed: u32,
    /// How many decisions changed the held action (a change of plan or a wait).
    pub replans: u32,
    /// The state the body was in after each decision. Kept because a deliberate
    /// episode is only useful to a person if it can be inspected: the action alone
    /// does not say whether the body was on the deck or falling beside it.
    pub trail: Vec<ThinkFrame>,
    /// What ended a dying episode, for the one-line report.
    pub death_cause: Option<&'static str>,
    /// One line in the controller's voice.
    pub note: String,
}

/// One decision of a deliberate episode, after it was taken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThinkFrame {
    pub frames: u32,
    pub x: f32,
    pub y: f32,
    pub action: AgentAction,
    pub stand: Option<usize>,
    pub aim: (f32, f32),
    pub grounded: bool,
    pub vx: f32,
    pub vy: f32,
}

impl ThinkReport {
    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
    }
}

/// Play a level with the deliberate controller.
///
/// Deterministic in `(level, seed, cfg)`: every branch is a pure function of the sim
/// snapshot and the fixed action order, and ties keep the earlier candidate.
pub fn play(level: &Level, seed: u64, cfg: ThinkConfig) -> ThinkReport {
    let cfg = cfg.sanitized();
    let mut sim = Sim::new(level, seed);
    let mut actions = Vec::new();
    let mut trail = Vec::new();
    let mut replans = 0;
    while !sim.outcome().is_finished() && actions.len() < cfg.max_decisions as usize {
        let aim = aim_point(&sim);
        let action = choose(&sim, aim, cfg);
        if action != actions.last().copied().unwrap_or(AgentAction::Walk) {
            replans += 1;
        }
        actions.push(action);
        for _ in 0..DECISION_TICKS {
            if sim.outcome().is_finished() {
                break;
            }
            sim.step(action);
        }
        trail.push(ThinkFrame {
            frames: sim.stats().frames,
            x: sim.player().x,
            y: sim.player().y,
            action,
            stand: sim.route_stand().or(sim.stats().best_stand),
            aim,
            grounded: sim.player().grounded,
            vx: sim.player().vx,
            vy: sim.player().vy,
        });
    }
    let stats = sim.stats();
    let note = if sim.outcome() == Outcome::Won {
        format!(
            "thought for {} decisions and reached the flag in {} frames",
            actions.len(),
            stats.frames
        )
    } else {
        format!(
            "thought for {} decisions; {} after {} frames at x {:.2}",
            actions.len(),
            match sim.outcome() {
                Outcome::Died => "died",
                Outcome::TimedOut => "ran out of level time",
                Outcome::Stuck => "stopped making progress",
                Outcome::Running | Outcome::Won => "stopped",
            },
            stats.frames,
            stats.best_x
        )
    };
    ThinkReport {
        actions,
        outcome: sim.outcome(),
        frames: stats.frames,
        best_x: stats.best_x,
        route_hops_closed: stats.route_hops_closed,
        replans,
        trail,
        death_cause: sim.death_cause().map(|c| c.as_str()),
        note,
    }
}

/// The world point the route wants the body at next.
///
/// The route graph's own aim point, with the flag as the fallback. A deck whose route
/// step is a ride aims at the body itself: that is how [`sim::reach::GoalRoutes`]
/// spells "hold still", and it is what makes a wait score as progress rather than as
/// standing in the wrong place.
pub fn aim_point(sim: &Sim) -> (f32, f32) {
    let level = sim.level();
    let player = sim.player();
    let cx = player.x + player.w * 0.5;
    let cy = player.y + player.h * 0.5;
    let goal = level
        .goals
        .first()
        .map(|g| (g.x + g.w * 0.5, g.y + g.h * 0.5));
    let stand = sim.route_stand().or(sim.stats().best_stand);
    let waypoint = stand.and_then(|stand| {
        level
            .routes()
            .next_waypoint(stand, cx, level.goals.first().copied())
    });
    waypoint.or(goal).unwrap_or((cx, cy))
}

/// Choose one decision block by searching the simulator.
fn choose(sim: &Sim, aim: (f32, f32), cfg: ThinkConfig) -> AgentAction {
    // A body standing on a mover is already being carried. The route graph's ride is
    // the one move whose action is *not* in the action set, so it is taken literally:
    // holding still on the deck is the correct press, and a search that tried to
    // "help" would walk off it.
    if let Some(stand) = sim.route_stand() {
        let p = sim.player();
        let cx = p.x + p.w * 0.5;
        let cy = p.y + p.h * 0.5;
        // A ride is the one step with no press: the route graph's aim is the body
        // itself. A deck that is a *landing* for a crossing can also carry a `ride`
        // edge, so the aim point — not merely being on a deck — is what says which
        // step is due. Waiting when the next step is a jump would never leave.
        let waiting = (cx - aim.0).abs() + (cy - aim.1).abs() < 1.0;
        if waiting && sim.level().routes().ride_from(stand).is_some() {
            // Brake rather than Walk: both have no horizontal input, but a body that
            // landed on the deck still carrying speed needs the stronger deceleration
            // or it drifts off the front while the deck travels the other way.
            return AgentAction::Brake;
        }
    }

    let mut frontier: Vec<Node> = vec![Node {
        sim: sim.clone(),
        actions: vec![None; cfg.horizon as usize],
        score: 0.0,
    }];
    let mut best: Option<Node> = None;

    for depth in 0..cfg.horizon as usize {
        let mut children: Vec<Node> = Vec::with_capacity(frontier.len() * AgentAction::COUNT);
        for node in &frontier {
            for action in AgentAction::ALL {
                let mut child = node.clone();
                child.sim = node.sim.clone();
                for _ in 0..DECISION_TICKS {
                    if child.sim.outcome().is_finished() {
                        break;
                    }
                    child.sim.step(action);
                }
                child.actions[depth] = Some(action);
                child.score = value(&child.sim, aim, cfg);
                // A finished episode is a leaf: it cannot be expanded further, and it
                // is exactly the evidence the search is looking for.
                if child.sim.outcome().is_finished() {
                    if best.as_ref().is_none_or(|held| child.score > held.score) {
                        best = Some(child);
                    }
                } else {
                    children.push(child);
                }
            }
        }
        if children.is_empty() {
            break;
        }
        frontier = prune(children, cfg.beam);
    }

    if let Some(node) = frontier
        .into_iter()
        .chain(best)
        .max_by(|a, b| a.score.total_cmp(&b.score))
    {
        if let Some(first) = node.actions.first().copied().flatten() {
            return first;
        }
    }
    AgentAction::Walk
}

#[derive(Clone)]
struct Node {
    sim: Sim,
    /// The action chosen at each depth; `None` beyond the leaves reached.
    actions: Vec<Option<AgentAction>>,
    score: f32,
}

/// The heuristic one snapshot is scored by.
///
/// A win is worth more than any reading, and a death more than any cost, so the
/// ordering between "won", "alive" and "dead" is categorical rather than a matter of
/// tuning the weights.
fn value(sim: &Sim, aim: (f32, f32), cfg: ThinkConfig) -> f32 {
    match sim.outcome() {
        Outcome::Won => return 1.0e9,
        Outcome::Died | Outcome::Stuck | Outcome::TimedOut => return -1.0e9,
        Outcome::Running => {}
    }
    let p = sim.player();
    let cx = p.x + p.w * 0.5;
    let cy = p.y + p.h * 0.5;
    let distance = (cx - aim.0).abs() + (cy - aim.1).abs();
    let stand = sim.route_stand().or(sim.stats().best_stand);
    let hops = stand
        .and_then(|stand| sim.level().routes().hops_to_flag(stand))
        .unwrap_or(0) as f32;
    let stats = sim.stats();
    // Checkpoints and decks are the sim's own record of a real landing; they stop a
    // candidate that is geometrically close but on the wrong platform from winning.
    let landings = stats.checkpoints as f32 + stats.decks as f32;
    let time = stats.frames as f32 * 0.005;
    // **Progress is the term that keeps a jump alive.** Distance to the aim point gets
    // *worse* while a body is in the air (it is higher and, at the start of a running
    // jump, barely further right), so a beam ordered by distance alone prunes the very
    // action that clears a spike before it lands. `progress_pixels` is the sim's
    // monotone best-ever approach to the flag, which a good jump improves immediately.
    let progress = stats.progress_pixels * 0.5;
    -distance * cfg.aim_weight * 0.25 + progress - hops * cfg.hop_weight + landings * 6.0 - time
}

/// Keep a beam without throwing away an option before its payoff.
///
/// The top-N-by-score rule is enough on open ground and wrong exactly where the
/// interesting decisions are: a jump's first blocks score worse than waiting (the body
/// is higher and no closer to the waypoint), so a pure beam drops every jump long
/// before the landing that would have justified it. Reserving the best node for each
/// **first action** is a cheap form of diversity that cannot lose an option the search
/// was asked to consider.
fn prune(children: Vec<Node>, beam: usize) -> Vec<Node> {
    let mut ranked: Vec<(usize, Node)> = children.into_iter().enumerate().collect();
    ranked.sort_by(|(_, a), (_, b)| b.score.total_cmp(&a.score));
    let mut keep = vec![false; ranked.len()];
    let mut seen = [false; AgentAction::COUNT];
    let mut kept = 0usize;
    for (index, node) in &ranked {
        if kept >= beam {
            break;
        }
        if let Some(Some(first)) = node.actions.first() {
            let slot = first.index();
            if !seen[slot] {
                seen[slot] = true;
                keep[*index] = true;
                kept += 1;
            }
        }
    }
    for (index, _) in &ranked {
        if kept >= beam {
            break;
        }
        if !keep[*index] {
            keep[*index] = true;
            kept += 1;
        }
    }
    let mut out: Vec<Node> = ranked
        .into_iter()
        .zip(keep)
        .filter_map(|((_, node), keep)| keep.then_some(node))
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(beam);
    out
}

/// A whole-route plan as a policy: the timed planner's own block actions.
///
/// This is the **deliberate** end of the controller: a plan is a claim about the
/// whole level, so it can wait out a deck's cycle or a turret's cooldown that no
/// short horizon can see. It is open loop by construction, and that is honest — the
/// plan was replayed to the flag before it was accepted, and the sim is deterministic.
/// `None` means the planner could not route the level, which is information rather
/// than an error: the caller can fall back to [`Driver::local`].
pub fn planned_actions(
    level: &Level,
    seed: u64,
    cfg: sim::plan::PlanConfig,
) -> Option<Vec<AgentAction>> {
    let outcome = sim::plan::plan_level(level, seed, cfg);
    let route = outcome.route?;
    let play = crate::route_play::timed_play(level, &route, &crate::fitness::EvalConfig::default());
    play.won().then(|| {
        play.decisions
            .iter()
            .map(|decision| decision.action)
            .collect()
    })
}

/// What a live window asks for one tick at a time.
///
/// A [`Session`](https://docs.rs/egui) in the app steps physics one tick at a time,
/// while this module's search decides once per [`DECISION_TICKS`]. Holding the chosen
/// action for the block is what makes the window's playback the same episode the
/// controller reasoned about; asking the search on every tick would both cost nine
/// times as much and change the answer inside a decision.
pub struct Driver {
    mode: Mode,
    cfg: ThinkConfig,
    action: AgentAction,
    remaining: u32,
    cursor: usize,
}

enum Mode {
    Planned(Vec<AgentAction>),
    Local,
}

impl Driver {
    /// The timer planner's route, when it found and replayed one.
    pub fn planned(level: &Level, seed: u64, cfg: sim::plan::PlanConfig) -> Option<Driver> {
        let actions = planned_actions(level, seed, cfg)?;
        Some(Driver {
            mode: Mode::Planned(actions),
            cfg: ThinkConfig::default(),
            action: AgentAction::Walk,
            remaining: 0,
            cursor: 0,
        })
    }

    /// The local model-based controller: no whole-route plan, just the next few blocks.
    pub fn local(cfg: ThinkConfig) -> Driver {
        Driver {
            mode: Mode::Local,
            cfg: cfg.sanitized(),
            action: AgentAction::Walk,
            remaining: 0,
            cursor: 0,
        }
    }

    /// Start the episode over.
    pub fn reset(&mut self) {
        self.action = AgentAction::Walk;
        self.remaining = 0;
        self.cursor = 0;
    }

    /// The action for this tick.
    pub fn next(&mut self, sim: &Sim) -> AgentAction {
        if let Mode::Planned(actions) = &self.mode {
            // The plan is indexed in decision blocks, and the body's whole frame count
            // is the clock: a session that started at tick zero on this level is the
            // episode the planner replayed.
            let block = (sim.stats().frames / DECISION_TICKS) as usize;
            return actions.get(block).copied().unwrap_or(AgentAction::Walk);
        }
        if self.remaining == 0 {
            self.action = choose(sim, aim_point(sim), self.cfg);
            self.remaining = DECISION_TICKS;
            self.cursor += 1;
        }
        self.remaining -= 1;
        self.action
    }

    /// Decisions taken so far, for a status line.
    pub fn decisions(&self) -> usize {
        self.cursor
    }
}

/// A short human-readable line for a CLI.
pub fn describe(level: &Level, report: &ThinkReport) -> String {
    format!(
        "{}: {} ({} decisions, {} frames, best x {:.1}{})",
        level.name,
        report.note,
        report.actions.len(),
        report.frames,
        report.best_x,
        if report.replans > 0 {
            format!(", {} replans", report.replans)
        } else {
            String::new()
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{flat_runway, gap_runway};

    fn quick() -> ThinkConfig {
        ThinkConfig {
            horizon: 12,
            beam: 16,
            ..ThinkConfig::default()
        }
    }

    #[test]
    fn it_walks_a_flat_runway() {
        let level = flat_runway("think-flat", 30, 27);
        let report = play(&level, 1, quick());
        assert!(report.won(), "{}", report.note);
        assert!(report.actions.len() > 1);
    }

    #[test]
    fn it_jumps_a_spike() {
        let src = crate::fixture::level_src(
            "think-spike",
            &[
                "..........",
                "..........",
                "..........",
                "....^.....",
                "##########",
            ],
            (1, 3),
            &[("goal", 8, 3)],
        );
        let level = Level::from_json_str(&src).expect("a spike level");
        let report = play(&level, 1, quick());
        assert!(report.won(), "{}", report.note);
    }

    #[test]
    fn it_jumps_a_pit() {
        let level = gap_runway("think-gap", 40, 36, &[(12, 3)]);
        let report = play(&level, 1, quick());
        assert!(report.won(), "{}", report.note);
    }

    #[test]
    fn a_planned_route_can_drive_the_same_episode() {
        let level = gap_runway("think-planned", 40, 36, &[(12, 3)]);
        let actions = planned_actions(&level, 1, sim::plan::PlanConfig::default())
            .expect("the timed planner can route a gap runway");
        assert!(!actions.is_empty());
        // The plan is the controller's whole route, so driving it tick by tick has to
        // reach the flag without consulting the local heuristic at all.
        let mut session = Sim::new(&level, 1);
        let mut driver = Driver::planned(&level, 1, sim::plan::PlanConfig::default())
            .expect("a driver from the same plan");
        while !session.outcome().is_finished() {
            let action = driver.next(&session);
            session.step(action);
        }
        assert_eq!(
            session.outcome(),
            Outcome::Won,
            "the planned driver stopped after {} decisions",
            driver.decisions()
        );
    }

    #[test]
    fn the_same_input_is_the_same_episode() {
        let level = gap_runway("think-repeat", 40, 36, &[(12, 3)]);
        let a = play(&level, 7, quick());
        let b = play(&level, 7, quick());
        assert_eq!(a, b);
    }

    #[test]
    fn the_weights_are_sanitized() {
        let cfg = ThinkConfig {
            horizon: 0,
            beam: 0,
            aim_weight: -1.0,
            hop_weight: -1.0,
            max_decisions: 0,
        }
        .sanitized();
        assert_eq!(cfg.horizon, 1);
        assert_eq!(cfg.beam, 1);
        assert_eq!(cfg.aim_weight, 0.0);
        assert_eq!(cfg.hop_weight, 0.0);
        assert_eq!(cfg.max_decisions, 1);
    }
}
