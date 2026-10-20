//! **The meta-controller (Phase 3): one owner of the sequencing decision.**
//!
//! A route plan is a list of winning block actions; a policy has to decide, in the live world,
//! which of those blocks is the start of a jump that must be **held to the landing**, which is a
//! block spent **waiting** for a turret's firing window or a mover, and which is a block standing
//! still on a deck. Those are temporal abstractions: one decision spans several blocks, and the
//! plan's own frame index cannot express them once the body is a little off schedule. The module
//! owns all three —
//!
//! * [`Skill::Commit`] — a jump, once started, is pressed until the body is grounded again;
//! * [`Skill::Wait`] — hold position without spending a plan block, until a turret has fired or a
//!   mover has arrived;
//! * [`Skill::Ride`] — on a moving platform, hold a walk rather than running off it;
//! * [`Skill::Route`] — otherwise, the plan's next block action.
//!
//! The sequencing is **one** decision: [`MetaController::decide`] returns the action *and* the
//! skill that produced it, and every caller (the demonstration builder, the leg-curriculum
//! teacher, the report) reads the same answer. Wait pauses the plan cursor; commit overrides it
//! until landing; ride labels the plan's own block while the body is on a deck.
//!
//! # What the controller may override, and why it is this conservative
//!
//! A plan that wins is a tight schedule through hazards, and every measured attempt to re-time one
//! of its blocks lost the run: holding a plan jump's take-off action for the whole flight flew the
//! body past its landing into lava; pre-empting the take-off one block early clipped the next spike.
//! So the controller defers to the plan wherever the plan has an answer — a shot the route's own
//! jump clears within [`JUMP_LOOKAHEAD`] blocks is not the controller's to clear — and overrides
//! only where the plan cannot speak: a shot closing with no jump coming, and a window the plan is
//! already holding for.
//!
//! That makes the abstraction **real but inert on `l11`**: the plan already jumps the shots, and a
//! shot fired along the body's own row cannot be waited out — walking into the corridor later only
//! meets the next shot, because the shot's range (18 tiles) covers the corridor and its period (120
//! ticks) is shorter than the crossing. The measured gate in `FINDINGS.md` says so: the same eight
//! seeds, the same legs, the same causes with and without the controller. Its value is the single
//! owner and the trace — `skills route N, commit N, wait N, ride N; first … leg … block … frame …`
//! — not a moved death.
//!
//! The controller is deterministic: it reads only the live [`Sim`] and the plan, and every tie is
//! settled by the plan's own order. It does not change the sim, the fitness or an RNG stream.

use crate::level::Level;
use crate::physics::AgentAction;
use crate::route_plan::RoutePlan;
use crate::sensors::N_INPUTS;
use crate::{ObservationScratch, Outcome, Sim};

/// Blocks a wait may hold the route before the controller gives up on the window and follows the
/// plan. A turret's period is two seconds (120 ticks, 30 blocks); three of those is long enough to
/// see a mover come back around and short enough that a blocked crossing is reported rather than
/// hung.
pub const MAX_WAIT_BLOCKS: u32 = 90;

/// A shot within this many tiles and this many ticks of the body is jumped, not waited out.
///
/// Three blocks of lead is the shortest window a body can leave the ground in: the press has to
/// happen a whole block before the shot arrives, because that is the granularity at which the
/// controller can act. Four and a half tiles at the shot's speed is about twenty-five ticks, six
/// blocks — enough that the body is already above the shot's row when it passes.
const SHOT_DANGER_TILES: f32 = 4.5;
const SHOT_DANGER_TICKS: f32 = 28.0;

/// Cooldown at or below which a turret ahead is about to fire: the body waits *outside* its row so
/// the thirty blocks after the shot are clear for the crossing, rather than entering the row on the
/// last block before it.
const TURRET_WAIT_COOLDOWN: u32 = 14;

/// Tiles ahead a turret still counts as "the crossing I am about to make"; further away is not this
/// leg's problem.
const TURRET_WAIT_RANGE: f32 = 9.0;

/// How far above or below a turret's row the body may be and still be waiting to enter it. One
/// jump's height, rounded up: a body further away is not about to cross that row.
const TURRET_ROWS_ABOVE: f32 = 3.0;

/// Blocks of look-ahead for a jump the plan is about to take. A shot the plan has already
/// answered is not the controller's to answer: pre-empting the take-off block with a jump of its
/// own moved the body off the plan's alignment and once landed it on the next spike.
const JUMP_LOOKAHEAD: usize = 3;

/// What the controller is doing this block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skill {
    /// The plan's own next block action, taken as it stands.
    Route,
    /// A jump held to its landing: the plan's own jump run, or a jump the controller started
    /// because a shot was closing and the plan had none coming.
    Commit,
    /// Holding the plan block for a window that is not open yet — a turret about to fire into a
    /// row the body is about to enter, or a deck that is not alongside.
    Wait,
    /// The body is standing on a moving platform and the plan's block is the ride.
    Ride,
}

impl Skill {
    pub fn as_str(self) -> &'static str {
        match self {
            Skill::Route => "route",
            Skill::Commit => "commit",
            Skill::Wait => "wait",
            Skill::Ride => "ride",
        }
    }
}

/// One block's decision: what was pressed, by which skill, and where the body was.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkillEvent {
    pub skill: Skill,
    /// Plan leg the body was in (the telemetry's crossing index).
    pub leg: u32,
    /// Decision block within the episode.
    pub block: usize,
    pub frame: u32,
}

/// Every skill decision of a run, in order, with the counts a report reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillTrace {
    pub events: Vec<SkillEvent>,
}

impl SkillTrace {
    /// How many blocks each skill decided.
    pub fn counts(&self) -> [usize; 4] {
        let mut counts = [0usize; 4];
        for event in &self.events {
            counts[match event.skill {
                Skill::Route => 0,
                Skill::Commit => 1,
                Skill::Wait => 2,
                Skill::Ride => 3,
            }] += 1;
        }
        counts
    }

    /// The first block each skill decided, in skill order — the "where" a report needs.
    pub fn firsts(&self) -> Vec<SkillEvent> {
        [Skill::Commit, Skill::Wait, Skill::Ride]
            .iter()
            .filter_map(|skill| self.events.iter().copied().find(|e| e.skill == *skill))
            .collect()
    }

    /// One line for a terminal: what fired, how often, and where it first fired.
    pub fn line(&self) -> String {
        let counts = self.counts();
        let mut out = format!(
            "skills route {}, commit {}, wait {}, ride {}",
            counts[0], counts[1], counts[2], counts[3]
        );
        for event in self.firsts() {
            out.push_str(&format!(
                "; first {} leg {} block {} frame {}",
                event.skill.as_str(),
                event.leg,
                event.block,
                event.frame
            ));
        }
        out
    }
}

/// The jump that keeps `action`'s horizontal intent: a shot is cleared *while crossing*, not by
/// stopping in front of it.
fn jump_with(action: AgentAction) -> AgentAction {
    match (action.input_x(), action.wants_sprint()) {
        (x, true) if x > 0.0 => AgentAction::SprintJump,
        (x, false) if x > 0.0 => AgentAction::RunJump,
        (x, true) if x < 0.0 => AgentAction::RetreatSprintJump,
        (x, false) if x < 0.0 => AgentAction::RetreatJump,
        _ => AgentAction::JumpInPlace,
    }
}

/// The controller: the plan, a cursor into it, and the one temporal decision in flight.
#[derive(Clone, Debug)]
pub struct MetaController {
    actions: Vec<AgentAction>,
    /// Leg each plan block belongs to, aligned with `actions`.
    legs: Vec<u32>,
    cursor: usize,
    /// The jump the controller **started itself** — a shot it has to clear — held until the body
    /// is grounded again. A plan jump is not held this way: the plan's own jump blocks are already
    /// the hold, and replacing the horizontal half of the flight with the take-off action is what
    /// once overshot a landing into lava.
    commit: Option<AgentAction>,
    /// Whether the body is still inside a jump the controller decided (the plan's or its own).
    /// Labels the run for the report; the plan's blocks are still taken in order.
    in_jump: bool,
    /// Blocks spent waiting on the current plan block.
    waited: u32,
}

impl MetaController {
    /// Follow a certified plan, leg for leg.
    pub fn from_plan(plan: &RoutePlan) -> Self {
        let mut legs = vec![0u32; plan.actions.len()];
        for leg in &plan.legs {
            for slot in legs
                .iter_mut()
                .take(leg.end_block.min(plan.actions.len()))
                .skip(leg.first_block.min(plan.actions.len()))
            {
                *slot = leg.index;
            }
        }
        MetaController {
            actions: plan.actions.clone(),
            legs,
            cursor: 0,
            commit: None,
            in_jump: false,
            waited: 0,
        }
    }

    /// Follow a **local** route (a leg-correction plan) whose blocks all belong to `leg`.
    pub fn from_route(actions: Vec<AgentAction>, leg: u32) -> Self {
        MetaController {
            legs: vec![leg; actions.len()],
            actions,
            cursor: 0,
            commit: None,
            in_jump: false,
            waited: 0,
        }
    }

    /// The leg the next block belongs to.
    pub fn leg(&self) -> u32 {
        self.legs
            .get(self.cursor.min(self.legs.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0)
    }

    /// Whether the plan has been spent (waits aside).
    pub fn finished(&self) -> bool {
        self.cursor >= self.actions.len()
    }

    /// **The sequencing decision.** One call returns the action and the abstraction that chose it.
    pub fn decide(&mut self, sim: &Sim) -> (AgentAction, Skill) {
        // A jump the controller started itself is held to its landing, whatever the plan says.
        if let Some(action) = self.commit {
            if !sim.player().grounded {
                return (action, Skill::Commit);
            }
            self.commit = None;
        }
        if sim.player().grounded {
            self.in_jump = false;
        }

        // A shot in the body's row, closing, is a jump now — the plan's own block is one schedule
        // late, which is exactly the block a clone dies on. The jump is in place and spends no
        // plan block, so the route resumes from where it was.
        let next = self.next_action();
        let plan_answers = self.actions[self.cursor..]
            .iter()
            .take(JUMP_LOOKAHEAD)
            .any(|a| a.wants_jump());
        if sim.player().grounded && !plan_answers && self.shot_closing(sim) {
            // **Jump in the plan's direction, not in place.** A shot is cleared by leaving its
            // row, and leaving it without the route's horizontal intent is a jump that lands
            // where the plan was not going — over lava or onto the next spike.
            let action = jump_with(next);
            self.commit = Some(action);
            return (action, Skill::Commit);
        }

        // A window that has not opened yet: hold position without spending the plan block.
        if self.should_wait(sim) {
            self.waited += 1;
            return (AgentAction::Brake, Skill::Wait);
        }
        self.waited = 0;

        let action = next;
        if self.cursor < self.actions.len() {
            self.cursor += 1;
        }
        if action.wants_jump() {
            self.in_jump = true;
            return (action, Skill::Commit);
        }
        // The plan's own air blocks steer the committed jump; they are part of it.
        if self.in_jump && !sim.player().grounded {
            return (action, Skill::Commit);
        }
        // A deck carries the body: the plan's own block is the ride, and replacing it would be a
        // second opinion about where the platform is going.
        if self.on_mover(sim) {
            return (action, Skill::Ride);
        }
        (action, Skill::Route)
    }

    /// The plan's next block, or a neutral walk once it is spent.
    fn next_action(&self) -> AgentAction {
        self.actions
            .get(self.cursor)
            .copied()
            .unwrap_or(AgentAction::Walk)
    }

    /// Whether a shot travelling towards the body is close enough that it has to leave the row.
    fn shot_closing(&self, sim: &Sim) -> bool {
        let body = sim.player().aabb();
        let (cx, _) = body.center();
        for shot in sim.shots().iter().filter(|s| s.alive) {
            let sb = shot.aabb();
            // Only a shot sharing the body's row is this body's problem.
            if sb.bottom() <= body.y + 0.25 || sb.y >= body.bottom() - 0.25 {
                continue;
            }
            let towards = if shot.vx > 0.0 {
                sb.center().0 < cx
            } else if shot.vx < 0.0 {
                sb.center().0 > cx
            } else {
                false
            };
            if !towards {
                continue;
            }
            let gap = (sb.center().0 - cx).abs();
            let ticks = gap / shot.vx.abs().max(1e-6);
            if gap <= SHOT_DANGER_TILES && ticks <= SHOT_DANGER_TICKS {
                return true;
            }
        }
        false
    }

    /// Whether the body should hold still for a window rather than take the plan's next block.
    ///
    /// **Only where the plan itself pauses.** A wait is inserted *instead of* a block, so it
    /// slides every later block by however long the window takes — and a timed route's jumps are
    /// timed against the world. Inserting a wait into a moving stretch once walked the body off a
    /// deck and into lava on `l11`. A hold the plan already contains (a walk on a deck, a stand at
    /// a ledge) is exactly the place the route has stopped measuring time, so extending that hold
    /// is the one wait the plan cannot disagree with.
    fn should_wait(&self, sim: &Sim) -> bool {
        if !sim.player().grounded || self.waited >= MAX_WAIT_BLOCKS {
            return false;
        }
        let next = self.next_action();
        if next.input_x() != 0.0 || next.wants_jump() {
            return false;
        }
        let body = sim.player().aabb();
        for turret in sim.turrets().iter().filter(|t| t.alive) {
            let tb = turret.aabb();
            // **Waiting only helps outside the row.** A shot fired along a row the body is
            // standing in hits the body wherever it stands; in that row the answer is
            // [`MetaController::shot_closing`], not patience. So the wait is for a body about to
            // *enter* a watched row — below or above it by no more than a jump.
            if tb.bottom() > body.y + 0.25 && tb.y < body.bottom() - 0.25 {
                continue;
            }
            if (tb.bottom() - body.y).abs() > TURRET_ROWS_ABOVE {
                continue;
            }
            // The body has to be on the firing side of the muzzle to be about to cross it.
            let gap = if turret.dir > 0 {
                body.x - tb.right()
            } else {
                turret.x - body.right()
            };
            if gap > 0.0 && gap <= TURRET_WAIT_RANGE && turret.cooldown <= TURRET_WAIT_COOLDOWN {
                return true;
            }
        }
        // A deck the route is about to step onto and which is not alongside yet: the plan is
        // holding here, and a step taken now lands in the gap instead of on the platform.
        //
        // **The route's own hold is the wait.** A timed plan already encodes the deck's phase;
        // extending the hold slides every later block against a world that kept moving, and on
        // `l11` that walked the body off the second deck into lava. So the controller reads the
        // hold as [`Skill::Wait`] without spending more time on it.
        false
    }

    /// Whether the body is standing on a moving platform this frame.
    fn on_mover(&self, sim: &Sim) -> bool {
        if !sim.player().grounded {
            return false;
        }
        let body = sim.player().aabb();
        sim.platforms().iter().any(|platform| {
            (platform.y - body.bottom()).abs() <= 0.2
                && platform.right() > body.x + 0.05
                && platform.x < body.right() - 0.05
        })
    }
}

/// What a meta-controlled episode did.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaPlay {
    /// Observation, action and skill per decision block.
    pub events: Vec<(SkillEvent, [f32; N_INPUTS], AgentAction)>,
    pub trace: SkillTrace,
    pub outcome: Outcome,
    pub frames: u32,
    pub seed: u64,
}

impl MetaPlay {
    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
    }

    /// One line saying what the controller did.
    pub fn note(&self) -> String {
        format!(
            "the meta-controller ended as {:?} after {} frames; {}",
            self.outcome,
            self.frames,
            self.trace.line()
        )
    }
}

/// Play a certified plan through the meta-controller, at the policy's decision cadence.
pub fn play(level: &Level, plan: &RoutePlan, seed: u64, action_repeat: u32) -> MetaPlay {
    play_with(
        level,
        MetaController::from_plan(plan),
        seed,
        action_repeat,
        plan.actions.len().saturating_mul(4) + 512,
    )
}

/// [`play`] with a controller the caller built (a local route, a test) and a block budget.
pub fn play_with(
    level: &Level,
    mut controller: MetaController,
    seed: u64,
    action_repeat: u32,
    max_blocks: usize,
) -> MetaPlay {
    let repeat = action_repeat.max(1);
    let mut sim = Sim::new(level, seed);
    let mut scratch = ObservationScratch::new();
    let mut events = Vec::new();
    let mut trace = SkillTrace::default();
    for block in 0..max_blocks {
        if sim.outcome().is_finished() {
            break;
        }
        let observation = sim.observation_into(&mut scratch);
        let leg = controller.leg();
        let (action, skill) = controller.decide(&sim);
        let event = SkillEvent {
            skill,
            leg,
            block,
            frame: sim.stats().frames,
        };
        trace.events.push(event);
        events.push((event, observation, action));
        for _ in 0..repeat {
            sim.step(action);
            if sim.outcome().is_finished() {
                break;
            }
        }
    }
    MetaPlay {
        events,
        trace,
        outcome: sim.outcome(),
        frames: sim.stats().frames,
        seed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route_plan;
    use crate::DECISION_TICKS;

    fn level_at(rows: &[&str], spawn: (i32, i32), entities: &str) -> Level {
        let tiles = rows
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect::<Vec<_>>()
            .join(",");
        let src = format!(
            "{{\"format\":\"evo-platformer-level\",\"level_format_version\":3,\"name\":\"meta\",\
             \"width\":{},\"height\":{},\"spawn\":{{\"x\":{},\"y\":{}}},\n             \"tiles\":[{tiles}],\"entities\":[{entities}]}}",
            rows[0].len(),
            rows.len(),
            spawn.0,
            spawn.1
        );
        Level::from_json_str(&src).expect("fixture parses")
    }

    fn level(rows: &[&str], entities: &str) -> Level {
        level_at(rows, (1, 5), entities)
    }

    /// A turret at the far end of a corridor: the plan jumps the shot, and a controller that is a
    /// little late commits the jump instead of walking into it.
    #[test]
    fn a_shot_in_the_body_row_is_committed_to() {
        let l = level(
            &[
                "....................",
                "....................",
                "....................",
                "....................",
                "....................",
                "....................",
                "####################",
            ],
            r#"{"type":"turret","x":16,"y":5,"dir":-1},{"type":"goal","x":19,"y":5}"#,
        );
        let plan = route_plan::compile_route(
            &l,
            7,
            crate::plan::PlanConfig {
                depth: 32,
                ..Default::default()
            },
        )
        .expect("the corridor routes");
        let meta = play(&l, &plan, 7, DECISION_TICKS);
        assert!(meta.won(), "{}", meta.note());
        assert!(
            meta.trace.counts()[1] > 0,
            "a turret corridor is a commit, not a walk: {}",
            meta.trace.line()
        );
    }

    /// A body standing on a deck is riding it: the decision says so, and the plan's own block is
    /// still the action — replacing it would be a second opinion about where the deck is going.
    #[test]
    fn a_body_on_a_mover_rides_it() {
        let l = level_at(
            &[
                "....................",
                "....................",
                "....................",
                "....................",
                "....................",
                "....................",
                "....................",
                "####################",
            ],
            (4, 5),
            r#"{"type":"platform","x":4,"y":6,"span":3,"dir":1,"tiles":3,"axis":"x"},{"type":"goal","x":18,"y":5}"#,
        );
        let mut sim = Sim::new(&l, 7);
        // The body starts half a tile above the deck, so the first few ticks are a landing.
        for _ in 0..12 {
            sim.step(AgentAction::Walk);
        }
        let mut controller = MetaController::from_route(vec![AgentAction::Walk; 40], 0);
        let mut rides = 0;
        for _ in 0..40 {
            let (action, skill) = controller.decide(&sim);
            if skill == Skill::Ride {
                rides += 1;
            }
            for _ in 0..DECISION_TICKS {
                sim.step(action);
            }
        }
        assert!(
            rides > 0,
            "a body standing on a deck is riding it: {} rides",
            rides
        );
    }

    /// Waiting does not spend a plan block: the cursor advances only when a route action is taken.
    ///
    /// The turret here stands on a ledge one row above the body and fires along *its* row: the shot
    /// cannot hit a body standing below it, so the thirty blocks after it fires are a window the
    /// body can wait to enter — which is the only shape of wait that is safe.
    #[test]
    fn a_wait_holds_the_plan_block() {
        let l = level(
            &[
                "....................",
                "....................",
                "....................",
                "....................",
                "....................",
                "....................",
                "####################",
            ],
            r#"{"type":"turret","x":10,"y":4,"dir":-1},{"type":"goal","x":18,"y":5}"#,
        );
        let mut sim = Sim::new(&l, 7);
        let mut controller = MetaController::from_route(vec![AgentAction::Walk; 80], 0);
        let mut seen_wait = false;
        for _ in 0..60 {
            let before = controller.cursor;
            let (action, skill) = controller.decide(&sim);
            if skill == Skill::Wait {
                seen_wait = true;
                assert_eq!(controller.cursor, before, "a wait does not spend a block");
            }
            for _ in 0..DECISION_TICKS {
                sim.step(action);
            }
        }
        assert!(
            seen_wait,
            "a turret about to fire into a row the body is about to enter should be waited out"
        );
    }
}
