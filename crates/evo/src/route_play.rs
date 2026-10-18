//! Walking a level's route: the oracle's graph, driven through the real episode loop.
//!
//! The reachability oracle answers "can this be beaten" by sweeping canonical moves with the sim's
//! own stepper, and the route graph ([`sim::reach::goal_routes`]) is that same sweep assembled into
//! a plan: stands, and the take-off that carries a body from one to the next. What neither does is
//! **walk** it — and the gap between "a route exists" and "a body can follow it" is exactly where a
//! reachability verdict can be wrong in the direction the owner hit by hand: a pad the oracle never
//! modelled, so a level whose only crossing was a pad was called unbeatable while the population
//! crossed it easily (`REVIEW.md` §1.12).
//!
//! So this module is the oracle's route replayed as a *player*. Every decision goes to
//! [`sim::Sim::step`] — the same loop training and play use — and the answer is an outcome rather
//! than a property of a graph. Two things come out of that:
//!
//! * **A verdict nobody has to take on trust.** `train-cli reach` says a level is beatable; a replay
//!   says a body walked it, and when it fails it names the hop it failed on. That is a different
//!   claim from the oracle's, which is the point of having both.
//! * **A demonstration.** The replay records the observation and the action at every decision
//!   block: (state, action) pairs a policy could have made, which is the material
//!   [`crate::imitate`] behaviour-clones a warm start out of.
//!
//! **It is a replay and not a solver.** It walks to the current hop's take-off position, presses the
//! oracle's own script ([`sim::reach::trajectory_action`] and [`sim::reach::climb_action`], the very
//! functions the sweep used), and re-reads the graph when it lands. When the body cannot make a hop
//! it says so and stops, because a silent failure would be worse than an honest one — this is the
//! instrument that checks the oracle's answers, so its own failures have to be legible.
//!
//! # Four things the walk revealed, and what the replay does about them
//!
//! Every one was found by walking a shipped level, and the first three are properties of the
//! *oracle* rather than of this module — which is the reason to have a replay at all.
//!
//! * **A canonical script is indexed in ticks, not in decisions.** `trajectory_action`'s short hold
//!   counts physics ticks and `climb_action`'s rhythm holds the jump for half an air time in ticks.
//!   A controller that pressed them per decision block (four ticks) held a climb's jump for eighty
//!   ticks instead of twenty, and the climb never left the ground — `l7-wall-school` and
//!   `l7-ascent`. So the scripts here run per tick, exactly as the oracle runs them, and only the
//!   *route* decisions (which hop to make) are made per block.
//! * **A take-off is a position *and* a speed.** A plan's `launch_vx` is part of the move, and a body
//!   that arrives carrying the walk it came in on is not the body the oracle swept — `l2-first-gap`'s
//!   staircase, where the right plan from the right column landed back where it started until the
//!   approach matched the plan's own speed.
//! * **A plan is validated at its own position, to the tick.** The sweep's take-off grid is a
//!   twentieth of a tile, and on a level whose crossing has no margin — the `l6-marathon` pit a run
//!   jump clears by one pixel of body — pressing a quarter of a tile early is the difference between
//!   the far ledge and the pit. So the press is triggered on the tick that reaches the position, not
//!   on the block that comes closest to it.
//! * **Take-off positions inside solid ground are not positions.** Found first in the walk and fixed
//!   in the oracle itself (`sim::reach::takeoff_fits`, the rule that moved `MOVE_SET_VERSION` to 3):
//!   the sweep used to place bodies inside a step and record the landing the physics ejected them
//!   onto, which handed the graph a "cheapest" move up a staircase that no body could make.
//!
//! The last consequence is visible in the trace rather than in the outcome: a stand can offer several
//! moves for one hop, the graph's own preference is not always executable, and an attempt that closes
//! no hop is remembered so the next one is tried instead of repeating it.
//!
//! **And one thing the walk cannot hand over.** The controller here is deliberately *tick*-precise —
//! it presses on the tick the body reaches its take-off, because on `l6-marathon` and `l8-long-jump`
//! that tick is the difference between the far ledge and the pit — while a trained policy decides once
//! per [`EvalConfig::action_repeat`] ticks and holds that arm in between. So the demonstration belongs
//! to a finer control grid than the policy can act on, and the clone `crate::imitate` fits from it
//! walks the route without being able to finish it at the shipped cadence (at `action_repeat = 1` the
//! same clone solves the level on every seed; that module's docs carry the measurement). The decisions
//! recorded below are still exactly what the walk did — they are simply what the walk did, on the
//! walk's own clock.
//!
//! **Determinism is the sim's, not a second implementation's.** A replay is a function of
//! `(level, seed)`: the controller reads only the live sim's public state, and the sim is already a
//! pure function of its own seed, so two replays of one level agree bit for bit — which is what makes
//! one replay a witness for every run that reads it.

use sim::physics::AgentAction;
use sim::reach::{
    climb_action, extract_platforms, simulate_jump_at, simulate_swim, simulate_vine_climb,
    swim_action, trajectory_action, vine_action, vine_run, GoalRoutes, JumpHold, LaunchSpeed,
    Platform, RouteMove, Takeoff,
};
use sim::{spawn_route_stand, Level, Outcome, Rect, Sim, PHYSICS};

use crate::fitness::EvalConfig;

/// Decision blocks an approach may fail to close any distance before its plan is abandoned.
///
/// See the module docs: a take-off position the body's box cannot occupy must be *abandoned* rather
/// than pressed from elsewhere, because a canonical trajectory is only evidence about the position it
/// was swept at. Three blocks (twelve ticks) is long enough that a body walking steadily never
/// registers, and short enough that the retry is cheap.
const BLOCKED_BLOCKS: u32 = 3;

/// How close the body's horizontal speed has to be to a plan's own launch speed before it is pressed.
///
/// One decision block of acceleration moves `vx` by about 0.04 tiles/tick, so this is the cadence's
/// own granularity: closer than that and the controller would be asked to hold a speed it can only
/// visit every fourth tick.
const VX_TOL: f32 = 0.04;

/// How close to the top of a fall a jumping press may be made. See the `ready` gate in the walk:
/// the oracle's take-off is swept from a body standing still on the surface, which is `vy = 0`.
const VY_TOL: f32 = 0.01;

/// Decision blocks a replay may spend without closing a route hop before it gives up.
///
/// Generous on purpose: a hop on a long stand is a walk of forty tiles, and the walk is 0.06 tiles a
/// tick — 660 ticks, 165 blocks — while a *stuck* body is one that is not closing hops at all. At the
/// default cadence this is about sixteen seconds of simulated time, which is longer than the slowest
/// hop in the shipped pack and far shorter than the episode budget.
const STALL_BLOCKS: u32 = 240;

/// Decision blocks the body may spend *at* a plan's take-off position while the oracle's own
/// simulator says the press would not clear the hop from where the body is.
///
/// See [`press_lands`]: the graph's take-off is a point, the body is a box on a four-tick cadence, and
/// the press therefore lands within half a block of its aim rather than on it. When that is too much,
/// the fix is not a better aim — it is to *ask the physics* whether the press in hand actually
/// clears the hop, and press at the first position where it does. Six blocks is a search of about
/// three tiles each way around the aim, which covers the aim error and stops well short of "keep
/// walking until something happens".
const PRESS_BLOCKS: u32 = 6;

/// One decision of a replay: what the body saw, and what it was asked to do.
///
/// The pair is the whole content of a demonstration, and it is recorded for **every** block rather
/// than only for the blocks the route's plan owned: a body in mid-flight is making the decisions that
/// keep the jump alive, and those are exactly the ones a behavioural clone has to learn.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub observation: [f32; sim::sensors::N_INPUTS],
    pub action: AgentAction,
}

/// What a replay did.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutePlay {
    /// The decisions, in order, one per decision block the body got to make.
    pub decisions: Vec<Decision>,
    pub outcome: Outcome,
    pub frames: u32,
    /// Route hops closed on the way (`hops_to_flag` falling while the body stood on a stand).
    pub hops_closed: u32,
    /// The route's own length in hops, when the graph had one: how much of it a win had to walk.
    pub route_hops: Option<u32>,
    /// One line saying what happened, in the terms a report reads — the failing hop included.
    pub note: String,
    /// Every hop the replay attempted and how it went: the stand it started from, the take-off it
    /// aimed at, and whether the attempt closed a hop, landed elsewhere, or was abandoned because the
    /// body's box cannot stand at that take-off position.
    ///
    /// This is the diagnostic half of the module: "this level cannot be walked" is useless without
    /// *which* crossing stopped a body, and a level checker that cannot name the hop is not a checker.
    /// One line per attempt, in order, ending with [`RoutePlay::note`].
    pub trace: Vec<String>,
}

impl RoutePlay {
    /// The body reached the flag, driven only by the oracle's own route.
    pub fn won(&self) -> bool {
        self.outcome == Outcome::Won
    }
}

/// A hop to make: where to stand, what to carry into it, and what to press.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Plan {
    stand: usize,
    target_x: f32,
    move_: RouteMove,
    /// The stand the graph says this move lands on, when it lands on one. Carried so the local
    /// sweep can be asked whether the *press in hand* reaches it (see [`press_lands`]).
    to: Option<usize>,
    /// Whether the move touches the flag, which is a landing in its own right.
    touches_flag: bool,
}

impl Plan {
    /// Whether the script asks for the jump to be held at all.
    ///
    /// A walk-off (`JumpHold::None`) and a climb are the two cases that do not: a walk-off has no jump
    /// in it, and a climb's holding is its own rhythm ([`climb_action`]).
    fn holds_jump(self) -> bool {
        match self.move_ {
            RouteMove::Jump(t) => t.hold != JumpHold::None,
            // A vine's "hold" is its whole climb and a dive is a jump; the field only seeds
            // `Active::hovering`, which the swim and vine scripts do not read.
            RouteMove::Climb { .. } | RouteMove::Vine { .. } => true,
            RouteMove::Swim { dive, .. } => dive,
        }
    }

    /// The horizontal speed the oracle launched this move with: what the body has to be carrying when
    /// the plan is pressed.
    fn desired_vx(self) -> f32 {
        match self.move_ {
            RouteMove::Jump(t) => t.launch_vx,
            // A climb starts flush against its wall from a standstill: the rhythm is swept from rest,
            // and a body still sliding past the wall is a different climb.
            RouteMove::Climb { .. } => 0.0,
            // A swim and a vine are entered from the aim at whatever speed the approach built:
            // both scripts take the body over from there, and a pool's own drag is the cap.
            RouteMove::Swim { .. } | RouteMove::Vine { .. } => 0.0,
        }
    }

    /// The identity a failed attempt is remembered by: which stand, which take-off position (to a
    /// twentieth of a tile, the grid's own granularity) and which move.
    fn key(self) -> (usize, i32, RouteMove) {
        (
            self.stand,
            (self.target_x * 20.0).round() as i32,
            self.move_,
        )
    }

    /// The arm that starts this move.
    fn press_action(self, ph: &sim::Physics) -> AgentAction {
        match self.move_ {
            RouteMove::Jump(t) => trajectory_action(&t, self.holds_jump(), 0, ph),
            RouteMove::Climb { dir } => climb_action(dir, 0, ph),
            RouteMove::Swim { dir, dive } => swim_action(dir, dive, false, 0),
            RouteMove::Vine { .. } => AgentAction::JumpInPlace,
        }
    }
}

/// A plan being executed: how far into its script the body is.
#[derive(Clone, Copy, Debug)]
struct Active {
    plan: Plan,
    /// Physics ticks since the press — the unit the canonical scripts are indexed in.
    ticks: u32,
    /// Whether the script is still asking for the jump to be held. Cleared when the body's vertical
    /// velocity turns non-negative, which is the oracle's own definition of the ascent ending.
    hovering: bool,
    /// The body has left the ground, so a later touchdown is a landing on this hop.
    airborne: bool,
    /// `hops_closed` when the attempt started, so a landing can say whether it bought anything.
    hops_at_start: u32,
    /// The vine's top row, for a vine climb's exit timing — a property of the level, read once
    /// when the plan is pressed.
    vine_top: Option<i32>,
    /// A vine climb has reached its exit and is pressing off it.
    exiting: bool,
}

/// Which stand the body is standing on, if it is standing on one.
///
/// Read from the live body rather than remembered from the plan, so a hop that lands somewhere else
/// than the graph said is *noticed* instead of assumed away: the next block re-plans from where the
/// body actually is.
fn stand_under(level: &Level, sim: &Sim) -> Option<usize> {
    let p = sim.player();
    if !p.grounded {
        return None;
    }
    let col = Level::tile_x(p.x + p.w * 0.5);
    let row = Level::tile_y(p.y + p.h + 0.05);
    level.routes().zone_at(col, row)
}

/// How far the body travels in one decision block: the granularity of the walk, as opposed to the
/// press.
fn block_travel(vx: f32, repeat: u32) -> f32 {
    (vx.abs() * repeat as f32).max(0.15)
}

/// Whether this tick is a candidate to press `plan`, given how close the body has come to its take-off
/// position so far.
///
/// **Either side of the aim, within one block.** A body cannot stop on an arbitrary sub-tile position —
/// the cadence moves it two fifths of a tile a block at run speed — so "press at the plan's position"
/// can only be honoured to within one block, and the walk has to choose which side of the target to be
/// wrong on. It used to wait for the distance to start growing again, which made every press a *late*
/// one, and that is wrong wherever the hop's window closes as the body runs: `l9-wait-and-climb`'s
/// first crossing is the worked example — aimed at 6.00 with a spike in column 7, the body pressed at
/// 6.15 and its leading edge clipped the spike that the press on the aim clears. So the question "does
/// this press clear the hop" is asked on the near side too. [`press_lands`] is the arbiter and it asks
/// the oracle's own simulator from the live position and speed, which is a stronger answer than the aim
/// alone; the guard still keeps the check from firing while the plan is being chosen from the far side
/// of the stand.
fn press_due(plan: Plan, x: f32, vx: f32, window: f32, closest: &mut f32) -> bool {
    let dx = (plan.target_x - x).abs();
    if dx > 1.0 {
        return false;
    }
    let was_closest = dx >= *closest;
    let near = *closest <= 1.0;
    *closest = if closest.is_finite() {
        (*closest).min(dx)
    } else {
        dx
    };
    let at_rest = vx.abs() < 0.02;
    if *closest <= 0.02 {
        return true;
    }
    near && (at_rest || was_closest || dx <= window)
}

/// **Does this press actually clear the hop, from *here*, at *this* speed?**
///
/// The graph's take-off is a point in a twentieth-of-a-tile grid and the body is a box on a four-tick
/// cadence, so the press lands within half a block of its aim rather than on it — and on a hop whose
/// margin is a fraction of a tile (a five-tile pit against a sprint jump, `l8-long-jump`), half a
/// block late is the pit. The fix is not a finer aim: it is to ask the oracle's own simulator the
/// question the aim was standing in for, with the body's *actual* position and speed, and press at
/// the first place the answer is yes.
///
/// What counts is **progress**, not the graph's own landing: the sweep here is at a position and a
/// speed the graph never tried, so it may well land somewhere else on the way — as long as it lands,
/// is not a hazard, and the landing stand is strictly closer to the flag than the one the body is
/// standing on. The replay re-reads the graph after every landing, so a better landing than the plan
/// promised is a win rather than a discrepancy.
///
/// A climb is not checked: its script is a rhythm held against a wall rather than a press at a
/// position, and the graph's own coordinates for it are the wall itself.
fn press_lands(
    level: &Level,
    platforms: &[Platform],
    routes: &GoalRoutes,
    plan: Plan,
    x: f32,
    surface_row: i32,
    vx: f32,
) -> bool {
    let takeoff = match plan.move_ {
        RouteMove::Jump(t) => Takeoff {
            dir: t.dir,
            hold: t.hold,
            speed: t.speed,
            // The speed the body is *carrying*, not the speed the plan asked for: the sweep here is
            // about the move that would actually happen.
            launch_vx: vx,
        },
        RouteMove::Climb { .. } => return true,
        // A swim and a vine are asked the same question the jumps are: does the script, run from
        // *here*, reach the stand the graph promised? A pool's far bank and a vine's top ledge are
        // ordinary landings, so the check is the oracle's own simulator rather than a special case.
        RouteMove::Swim { dir, dive } => {
            let traj = simulate_swim(level, platforms, x, surface_row, dir, dive);
            return traj.landed_at.and_then(|(c, r)| routes.zone_at(c, r)) == plan.to;
        }
        RouteMove::Vine { exit } => {
            let traj = simulate_vine_climb(level, platforms, x, surface_row, exit);
            return traj.landed_at.and_then(|(c, r)| routes.zone_at(c, r)) == plan.to;
        }
    };
    let traj = simulate_jump_at(level, platforms, x, surface_row as f32, &takeoff);
    if traj.hit_spike || traj.fell_out {
        return false;
    }
    if traj.touched_goal && plan.touches_flag {
        return true;
    }
    // The landing has to be the one the graph promised, not merely *a* landing: `l8-long-jump` is
    // the worked example of the looser rule being wrong — "anything closer to the flag" accepted a
    // press whose flight touched down on ground that is not a stand at all, and the body then walked
    // off the end of it into the pit. The graph's promise is the contract; this function's job is to
    // find a position that *keeps* it.
    let promised = plan.to;
    traj.landed_at
        .and_then(|(col, row)| routes.zone_at(col, row))
        .is_some_and(|landed| Some(landed) == promised && landed != plan.stand)
}

/// The action that walks the body toward its take-off position.
fn steer(dx: f32, tol: f32) -> AgentAction {
    if dx > tol {
        AgentAction::Run
    } else if dx < -tol {
        AgentAction::Retreat
    } else {
        AgentAction::Walk
    }
}

/// The action that moves the body's horizontal speed toward the plan's own launch speed.
///
/// Everything here is a *retard or accelerate* decision and none of it is a jump: the press happens
/// once the position and the speed are both right. Slowing down is the common case — a plan swept from
/// rest on a short runway needs the body stopped, and a body arriving from the far side of the stand
/// is still carrying the walk it came in on — so `Brake` (no horizontal input, hard deceleration) is
/// the arm a mis-speeding body gets, exactly as a player would use it.
fn match_speed(vx: f32, desired: f32, plan: Plan) -> AgentAction {
    if vx > desired {
        if desired < 0.0 {
            AgentAction::Retreat
        } else {
            AgentAction::Brake
        }
    } else {
        match plan.move_ {
            RouteMove::Jump(t) => match (t.dir.sign() > 0.0, t.speed == LaunchSpeed::Sprint) {
                (true, true) => AgentAction::Sprint,
                (false, true) => AgentAction::RetreatSprint,
                (true, false) => AgentAction::Run,
                (false, false) => AgentAction::Retreat,
            },
            // A climb's approach is a walk into its wall; the wall stops the body.
            RouteMove::Climb { dir } => {
                if dir.sign() > 0.0 {
                    AgentAction::Run
                } else {
                    AgentAction::Retreat
                }
            }
            // The swim and the vine take over from the aim: what they need of the approach is
            // only that the body *is* there, not how fast it is going.
            RouteMove::Swim { dir, .. } => {
                if dir.sign() > 0.0 {
                    AgentAction::Run
                } else {
                    AgentAction::Retreat
                }
            }
            RouteMove::Vine { .. } => AgentAction::Walk,
        }
    }
}

/// The plan to try next from `stand`, or `None` when walking is the move.
///
/// The graph's own choice ([`GoalRoutes::next_hop`], and [`GoalRoutes::flag_edge`] on the last hop)
/// comes first, because it is the move the waypoint sensor steers by and a replay that steered
/// differently from the sensor would be describing a route the population is not being pointed at.
/// The rest are the stand's other crossings that strictly reduce `hops_to_flag`, nearest first, so a
/// stand with several ways forward is worked through rather than repeated — and anything already tried
/// and failed is skipped unless it is the only thing left.
fn choose_plan(
    routes: &GoalRoutes,
    stand: usize,
    x: f32,
    goal: Option<Rect>,
    failed: &[(usize, i32, RouteMove)],
) -> Option<Plan> {
    let hops = routes.hops_to_flag(stand)?;
    if hops == 0 {
        // The stand is the flag's own ground: walking is the move.
        return None;
    }
    let preferred = if hops == 1 {
        routes
            .flag_edge(stand)
            .or_else(|| routes.next_hop(stand, x, goal))
    } else {
        routes.next_hop(stand, x, goal)
    };
    let mut candidates: Vec<(u8, u8, u32, f32, Plan)> = Vec::new();
    let mut push = |edge: &sim::reach::RouteEdge, preferred: bool| {
        let target_hops = edge.to.and_then(|j| routes.hops_to_flag(j));
        // A move that lands nowhere useful is only a candidate when it is the last hop's flag touch;
        // everything else has to strictly reduce the hop count to be progress at all.
        let reduces = target_hops.is_some_and(|h| h < hops) || (hops == 1 && edge.touches_flag);
        if !reduces {
            return;
        }
        let plan = Plan {
            stand,
            target_x: edge.takeoff_x,
            move_: edge.plan,
            to: edge.to,
            touches_flag: edge.touches_flag,
        };
        if candidates
            .iter()
            .any(|(_, _, _, _, p)| p.key() == plan.key())
        {
            return;
        }
        candidates.push((
            // Failed attempts last, however preferred they are: a plan that closed no hop is not
            // worth pressing again while the stand has another crossing to try.
            u8::from(failed.contains(&plan.key())),
            u8::from(!preferred),
            target_hops.unwrap_or(hops),
            (edge.takeoff_x - x).abs(),
            plan,
        ));
    };
    for edge in routes.crossings_from(stand) {
        push(edge, false);
    }
    if let Some(pick) = preferred {
        push(pick, true);
    }
    candidates.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.total_cmp(&b.3))
            .then(a.4.target_x.total_cmp(&b.4.target_x))
    });
    candidates.into_iter().next().map(|(_, _, _, _, plan)| plan)
}

/// Replay a level's route through the real episode loop.
///
/// A level with no route to its flag has nothing to walk — [`sim::reach::report`] is the instrument
/// that answers *that* — so the replay stops with a note saying so. Everything else — a hop the body
/// misses, a stand it cannot leave, a hazard in the way — ends the replay with the body's own outcome
/// and a note naming what happened.
pub fn replay(level: &Level, seed: u64, cfg: &EvalConfig) -> RoutePlay {
    let repeat = cfg.action_repeat.max(1);
    let ph = &PHYSICS;
    let mut sim = Sim::new(level, seed);
    let routes = level.routes();
    let goal = level.goals.first().copied();
    let goal_x = goal.map(|g| g.center().0).unwrap_or(level.width as f32);

    let mut decisions: Vec<Decision> = Vec::new();
    let mut held = AgentAction::Walk;
    // The plan being walked toward, once the route has chosen it.
    let mut pending: Option<Plan> = None;
    let mut active: Option<Active> = None;
    let mut failed: Vec<(usize, i32, RouteMove)> = Vec::new();
    let mut note = String::new();
    let mut hops_closed = 0u32;
    let mut best_hops: Option<u32> = None;
    let mut idle_blocks = 0u32;
    let mut approaching: Option<(usize, i32, RouteMove)> = None;
    let mut last_x = f32::NAN;
    let mut blocked_blocks = 0u32;
    // Blocks spent at a take-off position where the local sweep says the press would miss (see
    // `press_lands`).
    let mut press_blocks = 0u32;
    // The level's platforms, so the local sweep is the oracle's own geometry rather than a
    // hand-rolled approximation of it.
    let platforms = extract_platforms(level);
    // How close the body has been to the pending plan's take-off position, in tiles. Reset when the
    // route chooses a new plan: it is a property of the approach, not of the body.
    let mut closest_dx = f32::INFINITY;
    // The closest the body has been to the flag, so that a *walk* also counts as progress: the stall
    // guard is about a body that has stopped going anywhere, not only one that has stopped hopping.
    let mut closest_goal = f32::INFINITY;
    let route_hops = spawn_route_stand(level).and_then(|s| routes.hops_to_flag(s));
    let mut trace: Vec<String> = Vec::new();

    while !sim.outcome().is_finished() {
        let frames = sim.stats().frames;
        let on_block = frames % repeat == 0;
        // `x` is the body's **left edge**, because that is the position every take-off in the graph is
        // expressed in (`Takeoff` places a body's left edge at `takeoff_x`), and steering a *centre*
        // at it would leave the body a half-width short of the spot the move was swept from: on the
        // packed pack's pits, 0.3 of a tile is the difference between the far ledge and the pit.
        // `cx` is the centre, which is what the *route* questions are asked about — which stand the
        // body is standing on, and how far it is from the flag.
        let (x, cx, vx, vy, grounded) = {
            let p = sim.player();
            (p.x, p.x + p.w * 0.5, p.vx, p.vy, p.grounded)
        };

        // ---- a script in flight: the canonical arms, at the resolution they are written in ------
        if let Some(a) = active.as_mut() {
            if !grounded {
                a.airborne = true;
            }
            if a.airborne && grounded {
                let landed = stand_under(level, &sim);
                if hops_closed == a.hops_at_start {
                    failed.push(a.plan.key());
                    trace.push(format!(
                        "  tried {:?} from stand {} at x {:.2}: landed on {:?} and closed no hop",
                        a.plan.move_, a.plan.stand, a.plan.target_x, landed
                    ));
                } else {
                    trace.push(format!(
                        "  made {:?} from stand {} at x {:.2}: reached stand {:?} ({} hops closed)",
                        a.plan.move_, a.plan.stand, a.plan.target_x, landed, hops_closed
                    ));
                }
                active = None;
            } else {
                let on_vine = sim.player().on_vine;
                let in_water = sim.player().in_water;
                let feet = sim.player().y + sim.player().h;
                held = match a.plan.move_ {
                    RouteMove::Jump(t) => {
                        if a.hovering && vy >= 0.0 {
                            a.hovering = false;
                        }
                        trajectory_action(&t, a.hovering, a.ticks, ph)
                    }
                    RouteMove::Climb { dir } => climb_action(dir, a.ticks, ph),
                    RouteMove::Swim { dir, dive } => swim_action(dir, dive, in_water, a.ticks),
                    RouteMove::Vine { exit } => {
                        vine_action(exit, on_vine, feet, a.vine_top.unwrap_or(0), &mut a.exiting)
                    }
                };
                a.ticks += 1;
            }
        }

        // ---- nothing in flight: approach the chosen plan, and press it on the tick that arrives --
        if active.is_none() {
            if let Some(p) = pending {
                let desired = p.desired_vx();
                let tol = block_travel(vx, repeat);
                // **A jump is pressed from the ground.** The oracle sweeps a take-off from a body
                // standing at the surface with `vy = 0`, so a press made while the body is already
                // falling off the lip is a move the oracle never modelled — and it is the one
                // failure that looks like success up to the moment the body lands in the pit
                // (`l8-long-jump`, where the aim sits close enough to the stand's edge that the
                // block carrying the body past it is airborne). A climb is exempt: its script walks
                // *into* a wall and the body is on the ground for it by construction.
                let ready =
                    matches!(p.move_, RouteMove::Climb { .. }) || (grounded && vy.abs() <= VY_TOL);
                let due = ready
                    && press_due(p, x, vx, tol, &mut closest_dx)
                    && (vx - desired).abs() <= VX_TOL;
                // The aim is where the graph says to press; the physics says whether the press in
                // hand clears the hop from where the body is. Both have to agree.
                let row = routes.zone_bounds(p.stand).0;
                if due && press_lands(level, &platforms, routes, p, x, row, vx) {
                    let vine_top = match p.move_ {
                        RouteMove::Vine { .. } => {
                            let col = Level::tile_x(x + sim.player().w * 0.5);
                            vine_run(level, col, row, ph).map(|(_, top)| top)
                        }
                        _ => None,
                    };
                    active = Some(Active {
                        plan: p,
                        ticks: 0,
                        hovering: p.holds_jump(),
                        airborne: false,
                        hops_at_start: hops_closed,
                        vine_top,
                        exiting: false,
                    });
                    pending = None;
                    blocked_blocks = 0;
                    press_blocks = 0;
                    held = p.press_action(ph);
                    trace.push(format!(
                        "  pressing {:?} from stand {} at x {:.2} (aimed {:.2}) with vx {vx:.2}",
                        p.move_, p.stand, x, p.target_x
                    ));
                } else if due {
                    // The tick arrived but the move would not clear the hop from here: hold the
                    // press and keep the body near the aim, so the next block sweeps the next
                    // position. Bounded — a plan that never becomes executable is abandoned.
                    press_blocks += 1;
                    if press_blocks == 1 {
                        trace.push(format!(
                            "  held off {:?} from stand {} at x {:.2} with vx {vx:.2}: the local \
                             sweep does not clear the hop from here",
                            p.move_, p.stand, x
                        ));
                    }
                    if press_blocks > PRESS_BLOCKS {
                        failed.push(p.key());
                        pending = None;
                        approaching = None;
                        press_blocks = 0;
                        trace.push(format!(
                            "  abandoned {:?} from stand {} at x {:.2}: no press near the aim \
                             (x {:.2}) clears the hop",
                            p.move_, p.stand, x, p.target_x
                        ));
                        held = steer(goal_x - x, tol);
                    } else {
                        held = if (x - p.target_x).abs() > tol {
                            steer(p.target_x - x, tol)
                        } else {
                            match_speed(vx, desired, p)
                        };
                    }
                } else if (x - p.target_x).abs() > tol {
                    held = steer(p.target_x - x, tol);
                } else {
                    held = match_speed(vx, desired, p);
                }
                let to_goal = (goal_x - cx).abs();
                if to_goal < closest_goal {
                    closest_goal = to_goal;
                    idle_blocks = 0;
                }
            } else if on_block {
                // Idle on the ground: ask the route what the next hop is, work through the stand's
                // crossings, and abandon a take-off position the body cannot occupy.
                if last_x.is_nan() || (x - last_x).abs() > 0.02 {
                    blocked_blocks = 0;
                } else {
                    blocked_blocks += 1;
                }
                let to_goal = (goal_x - cx).abs();
                if to_goal < closest_goal {
                    closest_goal = to_goal;
                    idle_blocks = 0;
                }
                let stand = stand_under(level, &sim);
                if let Some(s) = stand {
                    match routes.hops_to_flag(s) {
                        Some(h) => {
                            if best_hops.is_none_or(|b| h < b) {
                                if best_hops.is_some() {
                                    hops_closed += 1;
                                }
                                best_hops = Some(h);
                                idle_blocks = 0;
                            }
                        }
                        // A stand the graph cannot route from is a hard stop: there is no hop to
                        // make, the body has not won, and waiting would only spend the budget.
                        None => {
                            note = format!(
                                "the body stood on a stand the route cannot leave (no crossing to \
                                 the flag from stand {s})"
                            );
                            break;
                        }
                    }
                }
                match stand.and_then(|s| choose_plan(routes, s, cx, goal, &failed)) {
                    Some(p) => {
                        if approaching == Some(p.key()) && blocked_blocks >= BLOCKED_BLOCKS {
                            // The body is as close to this take-off position as a body gets (its
                            // box against a riser), so the plan is abandoned rather than pressed
                            // from a position the oracle never swept it at.
                            failed.push(p.key());
                            approaching = None;
                            trace.push(format!(
                                "  abandoned {:?} from stand {} at x {:.2}: the body cannot get \
                                 closer than x {x:.2}",
                                p.move_, p.stand, p.target_x
                            ));
                            held = steer(goal_x - x, block_travel(vx, repeat));
                        } else {
                            approaching = Some(p.key());
                            pending = Some(p);
                            closest_dx = f32::INFINITY;
                            held = steer(p.target_x - x, block_travel(vx, repeat));
                        }
                    }
                    // Walking to a flag that is on this stand already.
                    None => {
                        approaching = None;
                        idle_blocks += 1;
                        held = steer(goal_x - x, block_travel(vx, repeat));
                    }
                }
            }

            if active.is_none() && pending.is_none() && on_block {
                idle_blocks += 1;
            }
        }
        last_x = x;

        if on_block {
            decisions.push(Decision {
                observation: sim.observation(),
                action: held,
            });
            if idle_blocks > STALL_BLOCKS {
                note = format!(
                    "the body stopped closing route hops after {idle_blocks} blocks (x = {x:.2}, vy \
                     = {vy:.2}), so the hop it was practising is not being made"
                );
                break;
            }
        }
        sim.step(held);
    }

    if note.is_empty() {
        let p = sim.player();
        note = match sim.outcome() {
            Outcome::Won => format!(
                "walked the route in {} frames over {hops_closed} hops",
                sim.stats().frames
            ),
            other => format!(
                "the replay ended as {other:?} at ({:.2}, {:.2}) after {hops_closed} hops and {} \
                 frames",
                p.x,
                p.y,
                sim.stats().frames
            ),
        };
    }

    trace.push(note.clone());
    RoutePlay {
        decisions,
        outcome: sim.outcome(),
        frames: sim.stats().frames,
        hops_closed,
        route_hops,
        note,
        trace,
    }
}
