//! Level validation: the jump-reachability oracle (§9).
//!
//! Answers "is this level beatable at all?" in milliseconds, before any training
//! budget is spent. Deliberately uses the **real** [`PlayerState::tick`] stepper
//! and the real collision query, so the oracle can never disagree with the sim
//! about how far a jump goes: it only disagrees about whether some *particular*
//! move is expressible in the action space, which is exactly the question.
//!
//! Live in `sim` rather than in the editor because it needs the physics bible and
//! the tile grid; the editor is just the first consumer.
//!
//! **Every move the game has is modelled here, or the oracle lies.** Sprint and wall
//! jumps are the two that forced this module to grow: a sprint changes how far a jump
//! goes *and* what runway it costs, and a wall climb is a rhythm rather than a jump, so
//! neither can be folded into the other's arithmetic. The reward for modelling them
//! properly is [`ReachOptions`]: the same report can be asked for with an ability
//! switched off, which turns "this level needs wall jumps" from a claim into a test.

use crate::level::{Level, Rect};
use crate::physics::{AgentAction, Physics, PlayerState, PHYSICS};

/// How long a single canonical jump trajectory may run.
pub const MAX_TRAJECTORY_TICKS: u32 = 240;

/// How long a canonical wall climb may run: ten seconds of wall time.
///
/// A climb is a *rhythm*, not a jump, so it needs a budget rather than a duration: at
/// roughly 2.4 tiles gained per 22-tick cycle it covers a 15-tile level's height in about
/// 150 ticks, and the cap is four times that so a tall shaft is not called unclimbable
/// because the oracle gave up early.
pub const MAX_CLIMB_TICKS: u32 = 600;

/// Which abilities the oracle is allowed to use.
///
/// Both are on by default: the question the oracle answers is "can the game be beaten",
/// and the game has sprint and wall jumps. Turning one off is how a level is checked for
/// *needing* it — a wall level should be unreachable without [`Self::wall_climb`], and
/// that claim is worth a test rather than a comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReachOptions {
    pub sprint: bool,
    pub wall_climb: bool,
}

impl Default for ReachOptions {
    fn default() -> Self {
        ReachOptions {
            sprint: true,
            wall_climb: true,
        }
    }
}

/// Sub-tile take-off offsets tried at every column.
///
/// An agent can stand anywhere, not just on a lattice: nothing in the sim snaps a
/// body to a tile boundary, so its left edge ranges over the *whole* column. L3's
/// spike patch is only clearable from a narrow band inside that range, and a
/// precision level is built out of jumps that are only clearable from one such
/// band, so a coarse sweep makes the oracle call beatable levels unbeatable — a
/// false negative that costs a level designer an hour and teaches them to distrust
/// the tool.
///
/// 0.05 is the granularity: fine enough that a band narrower than a twentieth of a
/// tile is the level's problem, not the oracle's. Cost is linear in this list
/// (20 entries, ~4x the old 5), and the whole sweep for a 60-tile level is still
/// milliseconds.
pub const TAKEOFF_OFFSETS: [f32; 20] = [
    0.0, 0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70, 0.75,
    0.80, 0.85, 0.90, 0.95,
];

/// A maximal horizontal run of solid tiles with open space above it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Platform {
    pub id: usize,
    /// Inclusive tile column range of the surface.
    pub x0: i32,
    pub x1: i32,
    /// Tile row of the solid surface. Its top edge is at world `y = surface_row`.
    pub surface_row: i32,
}

impl Platform {
    pub fn center_x(&self) -> f32 {
        (self.x0 + self.x1) as f32 * 0.5 + 0.5
    }

    /// World-space surface box, for drawing the validation overlay.
    pub fn surface_rect(&self) -> Rect {
        Rect::new(
            self.x0 as f32,
            self.surface_row as f32,
            (self.x1 - self.x0 + 1) as f32,
            1.0,
        )
    }

    pub fn contains_col(&self, col: i32) -> bool {
        col >= self.x0 && col <= self.x1
    }
}

/// How long the jump button is held during a canonical trajectory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpHold {
    /// No jump at all: walk or run off the edge.
    None,
    /// Released as early as the sim allows (`min_jump_hold`).
    Short,
    /// Held through the whole ascent.
    Full,
}

impl JumpHold {
    pub const JUMPING: [JumpHold; 2] = [JumpHold::Short, JumpHold::Full];

    pub fn name(self) -> &'static str {
        match self {
            JumpHold::None => "walk-off",
            JumpHold::Short => "short-jump",
            JumpHold::Full => "full-jump",
        }
    }
}

/// Take-off direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeoffDir {
    Left,
    Still,
    Right,
}

impl TakeoffDir {
    pub const ALL: [TakeoffDir; 3] = [TakeoffDir::Left, TakeoffDir::Still, TakeoffDir::Right];

    pub fn sign(self) -> f32 {
        match self {
            TakeoffDir::Left => -1.0,
            TakeoffDir::Still => 0.0,
            TakeoffDir::Right => 1.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TakeoffDir::Left => "left",
            TakeoffDir::Still => "still",
            TakeoffDir::Right => "right",
        }
    }
}

/// How fast a canonical trajectory is moving when it leaves the ground.
///
/// Speed is modelled as a **cost**, not a constant. A body cannot be sprinting just
/// because sprinting would clear the gap: it needs
/// [`Physics::runway_to_speed`] tiles of platform behind it, and
/// [`LaunchSpeed::achievable`] is that relation. Getting this wrong in the generous
/// direction is the expensive mistake — a level declared beatable by a sprint that has
/// nowhere to build up is a level the player, and the population, cannot beat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchSpeed {
    /// Standing start (a neutral jump).
    Rest,
    /// Full run speed, [`Physics::run_speed`].
    Run,
    /// Full sprint speed, [`Physics::sprint_speed`].
    Sprint,
}

impl LaunchSpeed {
    /// The two speeds that need `input_x` and a runway.
    pub const MOVING: [LaunchSpeed; 2] = [LaunchSpeed::Run, LaunchSpeed::Sprint];

    pub fn speed(self, ph: &Physics) -> f32 {
        match self {
            LaunchSpeed::Rest => 0.0,
            LaunchSpeed::Run => ph.run_speed,
            LaunchSpeed::Sprint => ph.sprint_speed,
        }
    }

    /// Runway needed to reach this speed from rest, in tiles.
    pub fn runway_tiles(self, ph: &Physics) -> f32 {
        ph.runway_to_speed(self.speed(ph))
    }

    /// The speed actually available after accelerating across `runway` tiles:
    /// `min(speed, sqrt(2·a·runway))`. With one tile of runway this gives a run speed of
    /// 0.1414, which is why every level the oracle passed before still passes: the old
    /// rule (`any runway at all means full run speed`) was the *pessimistic* end of this
    /// relation for a run and the optimistic one for nothing at all.
    pub fn achievable(self, ph: &Physics, runway: f32) -> f32 {
        let reachable = (2.0 * ph.run_accel * runway.max(0.0)).sqrt();
        reachable.min(self.speed(ph))
    }

    pub fn name(self) -> &'static str {
        match self {
            LaunchSpeed::Rest => "standing",
            LaunchSpeed::Run => "running",
            LaunchSpeed::Sprint => "sprinting",
        }
    }

    /// Whether a trajectory launched at this speed has to *keep asking* for it.
    ///
    /// The speed cap decays toward `run_speed` whenever the action stops requesting a
    /// sprint, so a jump launched at sprint speed with `run_jump` held is a sprint for ten
    /// ticks and then a run — the canonical trajectory has to hold the sprinting arm *of
    /// its own direction* (`sprint_jump` to the right, `retreat_sprint_jump` to the left)
    /// or it is not modelling the move it claims to.
    pub fn wants_sprint(self) -> bool {
        matches!(self, LaunchSpeed::Sprint)
    }
}

/// A canonical take-off: which way, how hard the jump button is held, which speed was
/// asked for, and how much of it the runway behind the take-off actually allows.
///
/// Kept as one value rather than as loose parameters because the four have to agree: a
/// sprint launch that does not hold the sprint action is a run with a faster first few
/// ticks, and a jump that claims a speed the platform cannot build is a level the oracle
/// would call beatable for the wrong reason.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Takeoff {
    pub dir: TakeoffDir,
    pub hold: JumpHold,
    pub speed: LaunchSpeed,
    /// Horizontal velocity the body carries into the jump, after runway.
    pub launch_vx: f32,
}

impl Takeoff {
    /// A take-off that is allowed the full nominal speed for `speed`.
    pub fn full(dir: TakeoffDir, hold: JumpHold, speed: LaunchSpeed, ph: &Physics) -> Takeoff {
        Takeoff {
            dir,
            hold,
            speed,
            launch_vx: dir.sign() * speed.speed(ph),
        }
    }

    /// A take-off from a platform that offers `runway` tiles behind it.
    pub fn with_runway(dir: TakeoffDir, hold: JumpHold, speed: LaunchSpeed, runway: f32, ph: &Physics) -> Takeoff {
        Takeoff {
            dir,
            hold,
            speed,
            launch_vx: dir.sign() * speed.achievable(ph, runway),
        }
    }
}

/// The outcome of one canonical jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trajectory {
    /// Platform the body came to rest on, if any.
    pub landed_on: Option<usize>,
    pub touched_goal: bool,
    pub hit_spike: bool,
    pub fell_out: bool,
    pub ticks: u32,
}

/// A single canonical jump at a single spot: enough to name the culprit in the
/// editor when a level is unreachable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CulpritJump {
    pub from: usize,
    pub takeoff_x: i32,
    /// Sub-tile offset of the body's left edge within the take-off column.
    pub takeoff_offset: f32,
    pub dir: TakeoffDir,
    pub hold: JumpHold,
    /// The speed the jump was launched with (and asked for).
    pub speed: LaunchSpeed,
    /// Aimed-at platform, when one exists to the goal-ward side.
    pub to: Option<usize>,
}

/// Answer to "can this level be beaten in principle?".
#[derive(Clone, Debug)]
pub struct ReachReport {
    pub platforms: Vec<Platform>,
    /// Platform the player spawns on.
    pub start: Option<usize>,
    /// Platform index containing the goal, when the goal sits on one.
    pub goal_platform: Option<usize>,
    pub goal_reached: bool,
    /// Index-aligned with `platforms`: reachable from the spawn platform.
    pub reached: Vec<bool>,
    /// The stuck jump to highlight when `goal_reached` is false.
    pub culprit: Option<CulpritJump>,
    pub diagnosis: Option<String>,
    /// Tiles covered by the reachable set, for a "could not get past here" marker.
    pub frontier_x: Option<i32>,
}

impl ReachReport {
    pub fn reachable_platforms(&self) -> usize {
        self.reached.iter().filter(|r| **r).count()
    }
}

/// Extract platforms from the tile grid (§9.1).
///
/// Ground counts as one long platform. Spikes are not platforms: they are lethal
/// and non-solid, so you can neither stand on one nor be carried by one.
pub fn extract_platforms(level: &Level) -> Vec<Platform> {
    let mut out = Vec::new();
    for row in 0..level.height {
        let mut x = 0;
        while x < level.width {
            let is_surface = level.is_solid(x, row) && !level.is_solid(x, row - 1);
            if !is_surface {
                x += 1;
                continue;
            }
            let start = x;
            let mut end = x;
            while end + 1 < level.width
                && level.is_solid(end + 1, row)
                && !level.is_solid(end + 1, row - 1)
            {
                end += 1;
            }
            out.push(Platform {
                id: out.len(),
                x0: start,
                x1: end,
                surface_row: row,
            });
            x = end + 1;
        }
    }
    out
}

fn platform_at(platforms: &[Platform], surface_row: i32, col: i32) -> Option<usize> {
    platforms
        .iter()
        .find(|p| p.surface_row == surface_row && p.contains_col(col))
        .map(|p| p.id)
}

/// The action a canonical trajectory issues on tick `t`.
///
/// **Both axes matter.** This used to choose by speed alone — `sprint_jump` for anything
/// launched at sprint speed, whatever direction it was launched in — which was correct
/// only while a sprint existed in one direction. On a leftward take-off it launched the
/// body at `-sprint_speed` and then held an action whose horizontal input is `+1`: the
/// model decelerated the very move it was testing, and a level whose sprint gap is on the
/// left was reported unbeatable while the sim, being asked for a move it did not have
/// either, agreed. See `AgentAction` for the gear that fixed both halves.
fn trajectory_action(takeoff: &Takeoff, hovering: bool, t: u32, ph: &Physics) -> AgentAction {
    let holding_jump = match takeoff.hold {
        JumpHold::None => false,
        // "Held through the ascent" is expressed as `hovering`, which the stepper
        // clears once vertical velocity turns non-negative.
        JumpHold::Full => hovering,
        JumpHold::Short => t < ph.min_jump_hold,
    };
    match (takeoff.dir, takeoff.speed.wants_sprint()) {
        (TakeoffDir::Right, true) => {
            if holding_jump {
                AgentAction::SprintJump
            } else {
                AgentAction::Sprint
            }
        }
        (TakeoffDir::Left, true) => {
            if holding_jump {
                AgentAction::RetreatSprintJump
            } else {
                AgentAction::RetreatSprint
            }
        }
        // A still take-off carries no horizontal speed (`dir.sign()` is 0), so there is no
        // gear to ask for: the neutral arms are the honest reading of "launched at rest".
        (TakeoffDir::Still, _) => {
            if holding_jump {
                AgentAction::JumpInPlace
            } else {
                AgentAction::Walk
            }
        }
        (TakeoffDir::Left, false) => {
            if holding_jump {
                AgentAction::RetreatJump
            } else {
                AgentAction::Retreat
            }
        }
        (TakeoffDir::Right, false) => {
            if holding_jump {
                AgentAction::RunJump
            } else {
                AgentAction::Run
            }
        }
    }
}

/// Simulate one canonical jump with the real physics stepper (§9.3), at the **full
/// nominal** speed for `speed`.
///
/// This is the form tests and explanations use ("can a running jump clear this?"). The
/// graph builder uses [`simulate_jump_at`] with [`LaunchSpeed::achievable`] instead,
/// because whether the body *has* the runway is the level's business, not the jump's.
///
/// * `takeoff_x` — tile column to launch from.
/// * `surface_row` — tile row the body stands on.
pub fn simulate_jump(
    level: &Level,
    platforms: &[Platform],
    takeoff_x: i32,
    surface_row: i32,
    dir: TakeoffDir,
    hold: JumpHold,
    speed: LaunchSpeed,
) -> Trajectory {
    simulate_jump_at(
        level,
        platforms,
        takeoff_x as f32 + (1.0 - PHYSICS.player_w) * 0.5,
        surface_row,
        &Takeoff::full(dir, hold, speed, &PHYSICS),
    )
}

/// As [`simulate_jump`], but with the body's left edge placed at an exact x and the exact
/// horizontal speed it carries into the take-off, so callers can sweep both the sub-tile
/// positions [`TAKEOFF_OFFSETS`] covers and the runway-dependent launch speeds.
pub fn simulate_jump_at(
    level: &Level,
    platforms: &[Platform],
    takeoff_x: f32,
    surface_row: i32,
    takeoff: &Takeoff,
) -> Trajectory {
    let ph = &PHYSICS;
    let mut player = PlayerState::spawn(level, ph);
    player.w = ph.player_w;
    player.h = ph.player_h;
    player.x = takeoff_x;
    player.y = surface_row as f32 - ph.player_h;
    player.vx = takeoff.launch_vx;
    // The cap has to come along with the speed: a body launched at sprint speed with a
    // running cap would have its launch clamped away on the very first tick, and the
    // oracle would report the sprint jump as a running one.
    player.set_speed_cap(takeoff.launch_vx.abs().max(ph.run_speed));
    player.vy = 0.0;
    // The body is authored standing on `surface_row`, which is by construction a
    // surface tile, so a jump is legal on tick zero.
    player.grounded = true;

    let mut hover = takeoff.hold != JumpHold::None;
    // A "landing" only counts once the body has actually left the ground, so
    // walking off an edge is not misreported as landing on the take-off platform
    // during the ticks it takes to clear the edge.
    let mut airborne = false;
    for t in 0..MAX_TRAJECTORY_TICKS {
        let action = trajectory_action(takeoff, hover, t, ph);
        let contact = player.tick(level, ph, action);
        if player.vy >= 0.0 {
            hover = false;
        }
        if !player.grounded {
            airborne = true;
        }
        if let Some(end) = trajectory_terminal(level, &player, &contact, t + 1) {
            return end;
        }
        if contact.landed && airborne {
            // Landing back on the take-off platform with no forward progress is
            // not an edge; it is reported as "landed here" and filtered by BFS.
            return landed_trajectory(platforms, &player, t + 1);
        }
    }
    Trajectory {
        landed_on: None,
        touched_goal: false,
        hit_spike: false,
        fell_out: false,
        ticks: MAX_TRAJECTORY_TICKS,
    }
}

/// The outcome of a body that has just touched down, after having been airborne.
///
/// The body lands as a *box*, not as a point. Asking which platform is under the body's
/// centre says a body standing with its feet on the last column of a ledge is in mid-air —
/// and that false negative is expensive in both directions: it hides the ledge a climb
/// ends on, and it is the same edge-straddling case every precision jump out of a pit
/// lands in. The rule is the sim's own (`PlayerState::spawn`): supported if *any* column
/// the body spans is solid at the row below its feet.
fn landed_trajectory(platforms: &[Platform], player: &PlayerState, ticks: u32) -> Trajectory {
    let row = (player.y + player.h).floor() as i32;
    let (c0, c1) = crate::physics::tile_span(player.x, player.w);
    Trajectory {
        landed_on: (c0..=c1).find_map(|c| platform_at(platforms, row, c)),
        touched_goal: false,
        hit_spike: false,
        fell_out: false,
        ticks,
    }
}

/// The ways a trajectory can end other than by landing: hazard, flag, or the void.
///
/// Shared by the jump and the wall climb so the two cannot disagree about what stops a
/// body — the one thing that would make the oracle's answer depend on *how* the body got
/// somewhere rather than where it is.
fn trajectory_terminal(
    level: &Level,
    player: &PlayerState,
    contact: &crate::physics::TickContact,
    ticks: u32,
) -> Option<Trajectory> {
    if contact.touched_spike {
        return Some(Trajectory {
            landed_on: None,
            touched_goal: false,
            hit_spike: true,
            fell_out: false,
            ticks,
        });
    }
    if level.goals.iter().any(|g| player.aabb().overlaps(g)) {
        return Some(Trajectory {
            landed_on: None,
            touched_goal: true,
            hit_spike: false,
            fell_out: false,
            ticks,
        });
    }
    if player.y > level.height as f32 {
        return Some(Trajectory {
            landed_on: None,
            touched_goal: false,
            hit_spike: false,
            fell_out: true,
            ticks,
        });
    }
    None
}

/// Simulate a canonical **wall climb**: press into a wall, and jump every time the body is
/// clinging to it.
///
/// This is the move wall jumps exist for, and the reason the oracle cannot answer "is the
/// wall short enough" arithmetically: a climb is a *rhythm*, and whether it gains height
/// depends on the away-push being recoverable inside the rise, on the wall's height, and on
/// what is above it. Only the real stepper knows, so the climb is scripted exactly as the
/// jump edges are scripted jumps — and because it is scripted out of real actions from a
/// real standing position, any edge it reports is a move the game really has.
///
/// The script is deliberately minimal — one direction, held on a fixed period — because it
/// has to stay explainable. A script clever enough to climb anything would be a policy, and
/// the oracle would be answering a different question.
///
/// * `start_x` — the body's left edge, flush against the wall.
/// * `surface_row` — the platform row it starts standing on.
/// * `dir` — the side the wall is on: the direction the script presses.
pub fn simulate_wall_climb(
    level: &Level,
    platforms: &[Platform],
    start_x: f32,
    surface_row: i32,
    dir: TakeoffDir,
) -> Trajectory {
    let ph = &PHYSICS;
    let (jump, press) = match dir {
        TakeoffDir::Left => (AgentAction::RetreatJump, AgentAction::Retreat),
        TakeoffDir::Right => (AgentAction::RunJump, AgentAction::Run),
        TakeoffDir::Still => (AgentAction::JumpInPlace, AgentAction::Walk),
    };
    // Hold through the ascent, then release for two ticks. The pause is what makes every
    // press a *fresh* one, which is what an edge-triggered jump needs, and it is short
    // enough that the body has barely started sliding again when the next wall jump fires.
    let hold = (ph.air_time_ticks() * 0.5).ceil() as u32;
    let period = hold + 2;

    let mut player = PlayerState::spawn(level, ph);
    player.w = ph.player_w;
    player.h = ph.player_h;
    player.x = start_x;
    player.y = surface_row as f32 - ph.player_h;
    // The caller chose this spot because the platform is under it, so the body starts
    // standing — exactly as `simulate_jump_at` does for a take-off.
    player.grounded = true;

    let mut airborne = false;
    for t in 0..MAX_CLIMB_TICKS {
        let action = if t % period < hold { jump } else { press };
        let contact = player.tick(level, ph, action);
        if !player.grounded {
            airborne = true;
        }
        if let Some(end) = trajectory_terminal(level, &player, &contact, t + 1) {
            return end;
        }
        if contact.landed && airborne {
            return landed_trajectory(platforms, &player, t + 1);
        }
    }
    Trajectory {
        landed_on: None,
        touched_goal: false,
        hit_spike: false,
        fell_out: false,
        ticks: MAX_CLIMB_TICKS,
    }
}

/// Runway, in tiles, available *behind* a take-off at `col` on platform `p`.
///
/// Behind, not beyond: the body is already moving when it reaches the ledge it launches
/// from. Getting this backwards silently made every pit uncrossable, because the
/// platform's outermost column — the only one you can clear a gap from — is exactly the
/// column with no platform to its outside.
fn runway_behind(p: &Platform, col: i32, dir: TakeoffDir) -> f32 {
    match dir {
        TakeoffDir::Right => (col - p.x0) as f32,
        TakeoffDir::Left => (p.x1 - col) as f32,
        TakeoffDir::Still => 0.0,
    }
}

/// Where a canonical wall climb can start from `p`: either end, when a wall stands there.
///
/// A wall in the middle of a surface splits it into two platforms (`extract_platforms`
/// treats the wall's own column as a surface of its own), so a wall beside a platform is
/// always at one of its ends.
fn wall_climb_starts(level: &Level, p: &Platform, ph: &Physics) -> Vec<(f32, TakeoffDir)> {
    let mut out = Vec::new();
    let r = p.surface_row;
    // Right: the wall's column is the first one past the platform, and the body starts
    // flush against its face.
    if level.is_solid(p.x1 + 1, r - 1) {
        out.push(((p.x1 + 1) as f32 - ph.player_w, TakeoffDir::Right));
    }
    // Left: the body's left edge is the platform's first column, so the wall's column is
    // the one before it.
    if level.is_solid(p.x0 - 1, r - 1) {
        out.push((p.x0 as f32, TakeoffDir::Left));
    }
    out
}

/// Every canonical move out of `platform` (§9.3): jumps at every sub-tile take-off point
/// and launch speed, walk-offs at the ends, and wall climbs at any end that has a wall.
fn edges_from(
    level: &Level,
    platforms: &[Platform],
    p: &Platform,
    options: &ReachOptions,
) -> Vec<(Trajectory, CulpritJump)> {
    let ph = &PHYSICS;
    let mut out = Vec::new();
    let cols = p.x0..=p.x1;
    for col in cols {
      for &offset in &TAKEOFF_OFFSETS {
        let x = col as f32 + offset;
        for dir in TakeoffDir::ALL {
            let runway = runway_behind(p, col, dir);
            let holds: &[JumpHold] = if dir == TakeoffDir::Still {
                &[JumpHold::Short, JumpHold::Full]
            } else {
                &JumpHold::JUMPING
            };
            let speeds: &[LaunchSpeed] = if dir == TakeoffDir::Still {
                &[LaunchSpeed::Rest]
            } else if options.sprint {
                &LaunchSpeed::MOVING
            } else {
                &[LaunchSpeed::Run]
            };
            for hold in holds {
                for speed in speeds {
                    let takeoff = Takeoff::with_runway(dir, *hold, *speed, runway, ph);
                    let traj = simulate_jump_at(level, platforms, x, p.surface_row, &takeoff);
                    out.push((traj, CulpritJump {
                        from: p.id,
                        takeoff_x: col,
                        takeoff_offset: offset,
                        dir,
                        hold: *hold,
                        speed: *speed,
                        to: traj.landed_on.filter(|q| *q != p.id),
                    }));
                }
            }
            // Walking off the edge, only from the ends.
            let at_edge = col == p.x0 || col == p.x1;
            if at_edge && dir != TakeoffDir::Still {
                let takeoff = Takeoff::with_runway(dir, JumpHold::None, LaunchSpeed::Run, runway, ph);
                let traj = simulate_jump_at(level, platforms, x, p.surface_row, &takeoff);
                out.push((traj, CulpritJump {
                    from: p.id,
                    takeoff_x: col,
                    takeoff_offset: offset,
                    dir,
                    hold: JumpHold::None,
                    speed: LaunchSpeed::Run,
                    to: traj.landed_on.filter(|q| *q != p.id),
                }));
            }
        }
      }
    }
    if options.wall_climb {
        for (start_x, dir) in wall_climb_starts(level, p, ph) {
            let traj = simulate_wall_climb(level, platforms, start_x, p.surface_row, dir);
            out.push((traj, CulpritJump {
                from: p.id,
                takeoff_x: start_x.floor() as i32,
                takeoff_offset: start_x.fract(),
                dir,
                // A climb is a sequence of full-height wall jumps, which is what the
                // report should say it was rather than naming a single hold.
                hold: JumpHold::Full,
                speed: LaunchSpeed::Run,
                to: traj.landed_on.filter(|q| *q != p.id),
            }));
        }
    }
    out
}

/// Run the oracle: platform graph + BFS from the spawn platform to the goal (§9.4).
///
/// Uses every ability the game has. [`report_with`] is the same question with abilities
/// switched off, which is how a level is checked for *needing* one.
pub fn report(level: &Level) -> ReachReport {
    report_with(level, ReachOptions::default())
}

/// As [`report`], with the abilities under the caller's control.
pub fn report_with(level: &Level, options: ReachOptions) -> ReachReport {
    let platforms = extract_platforms(level);

    let spawn_col = (level.spawn.x + level.spawn.w * 0.5).floor() as i32;
    let spawn_surface_row = (level.spawn.y + level.spawn.h).floor() as i32;
    let start = platform_at(&platforms, spawn_surface_row, spawn_col);

    // The goal, and the column it stands in — `None` for a document that has lost its flag
    // ([`Level::remove_goal_at`] allows that while editing). The oracle is asked about a
    // level every frame in the editor, so "there is a flag" has to be answered rather than
    // assumed: indexing an empty goal list here would be a panic, not a verdict.
    let goal: Option<(Rect, i32)> = level
        .goals
        .first()
        .map(|g| (*g, (g.x + g.w * 0.5).floor() as i32));
    let goal_platform = goal.and_then(|(g, col)| {
        platforms
            .iter()
            .find(|p| p.contains_col(col) && (p.surface_row as f32) >= g.y)
            .map(|p| p.id)
    });

    let mut reached = vec![false; platforms.len()];
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); platforms.len()];
    let mut touched_goal_from: Vec<bool> = vec![false; platforms.len()];
    let mut all_edges: Vec<Vec<CulpritJump>> = vec![Vec::new(); platforms.len()];

    for p in &platforms {
        for (traj, culprit) in edges_from(level, &platforms, p, &options) {
            all_edges[p.id].push(culprit);
            if traj.touched_goal {
                touched_goal_from[p.id] = true;
            }
            if let Some(q) = traj.landed_on {
                if q != p.id && !adjacency[p.id].contains(&q) {
                    adjacency[p.id].push(q);
                }
            }
        }
    }

    let mut goal_reached = false;
    if let Some(start) = start {
        // BFS. Deterministic: neighbours are visited in insertion order.
        let mut queue = std::collections::VecDeque::new();
        reached[start] = true;
        queue.push_back(start);
        while let Some(id) = queue.pop_front() {
            if touched_goal_from[id] {
                goal_reached = true;
            }
            if let (Some((_, col)), Some(goal_platform)) = (goal, goal_platform) {
                if id == goal_platform && platforms[id].contains_col(col) {
                    goal_reached = true;
                }
            }
            for &n in &adjacency[id] {
                if !reached[n] {
                    reached[n] = true;
                    queue.push_back(n);
                }
            }
        }
    }

    let (culprit, diagnosis, frontier_x) = if goal_reached {
        (None, None, None)
    } else {
        match goal {
            Some((_, col)) => diagnose(&platforms, &reached, &all_edges, col, &options),
            // Nothing to reach *because there is no flag* — the honest reason, and not one to
            // dress up as a jump that cannot be made.
            None => (None, Some("the level has no goal flag".into()), None),
        }
    };

    ReachReport {
        platforms,
        start,
        goal_platform,
        goal_reached,
        reached,
        culprit,
        diagnosis,
        frontier_x,
    }
}

/// Name the first jump that cannot be made: from the reachable platform nearest
/// the goal, describe why nothing further is possible (§9.4).
fn diagnose(
    platforms: &[Platform],
    reached: &[bool],
    edges: &[Vec<CulpritJump>],
    goal_col: i32,
    options: &ReachOptions,
) -> (Option<CulpritJump>, Option<String>, Option<i32>) {
    if platforms.is_empty() {
        return (None, Some("level has no solid ground at all".into()), None);
    }

    // The reachable platform whose surface gets closest to the goal.
    let mut best: Option<&Platform> = None;
    for p in platforms.iter().filter(|p| reached[p.id]) {
        let better = match best {
            None => true,
            Some(b) => (goal_col - p.x1).abs() < (goal_col - b.x1).abs(),
        };
        if better {
            best = Some(p);
        }
    }
    let Some(from) = best else {
        return (
            None,
            Some("the spawn tile is not on any solid platform".into()),
            None,
        );
    };

    // Prefer a failing edge that aimed forward; otherwise report the nearest
    // forward target on its own.
    let forward: Vec<&CulpritJump> = edges[from.id]
        .iter()
        .filter(|c| match c.dir {
            TakeoffDir::Left => goal_col < from.x0,
            TakeoffDir::Right => goal_col > from.x1,
            TakeoffDir::Still => false,
        })
        .collect();

    let target = platforms
        .iter()
        .filter(|p| p.surface_row <= from.surface_row && p.x0 > from.x1)
        .min_by_key(|p| p.x0 - from.x1);

    let message = match target {
        Some(t) => {
            let gap = (t.x0 - from.x1 - 1) as f32;
            let rise = (from.surface_row - t.surface_row) as f32;
            let ph = &PHYSICS;
            // The platform's own length is the runway a sprint would have to build in.
            let runway = (from.x1 - from.x0) as f32;
            let sprint_runway = ph.runway_to_speed(ph.sprint_speed);
            if rise > ph.discrete_max_jump_height() {
                format!(
                    "wall too tall: the next surface rises {rise:.0} tiles, but a full jump reaches only {:.2} tiles and a wall jump climbs the same{} — lower the top surface, or leave a wall beside it to climb",
                    ph.discrete_max_jump_height(),
                    if options.wall_climb {
                        ", and no wall beside the reachable set is climbable to it"
                    } else {
                        " (wall climbs were not modelled for this check)"
                    }
                )
            } else if gap > ph.sprint_jump_distance() {
                format!(
                    "gap too wide: {gap:.0} empty tiles to the next surface; a running jump covers {:.2} tiles and even a sprinting one only {:.2} (and a sprint needs {sprint_runway:.2} tiles of run-up)",
                    ph.max_jump_distance(),
                    ph.sprint_jump_distance()
                )
            } else if gap > ph.max_jump_distance() {
                // Only a sprint clears this. Either the platform is long enough to build one
                // and the *landing* is what fails, or it is not long enough at all — and the
                // designer is owed the difference, because it is the difference between
                // "move the far ledge" and "give me more run-up".
                if !options.sprint {
                    format!(
                        "gap too wide to run: {gap:.0} empty tiles to the next surface; a running jump covers {:.2} and a sprinting one would cover {:.2}, but this check was run without sprints",
                        ph.max_jump_distance(),
                        ph.sprint_jump_distance()
                    )
                } else if runway >= sprint_runway {
                    format!(
                        "no take-off from platform {} clears the {gap:.0}-tile gap: a sprinting jump covers {:.2} tiles and must land past it",
                        from.id,
                        ph.sprint_jump_distance()
                    )
                } else {
                    format!(
                        "gap too wide to run: {gap:.0} empty tiles to the next surface; a running jump covers {:.2} and a sprinting one {:.2}, but a sprint needs {sprint_runway:.2} tiles of run-up and platform {} is {runway:.0} long",
                        ph.max_jump_distance(),
                        ph.sprint_jump_distance(),
                        from.id
                    )
                }
            } else {
                format!(
                    "no canonical move from platform {} reaches the next surface: {gap:.0}-tile gap, {rise:.0}-tile rise, {} take-off points tried",
                    from.id,
                    edges[from.id].len()
                )
            }
        }
        None => "no further surface exists to the goal-ward side of the reachable set".into(),
    };

    let culprit = forward
        .first()
        .map(|c| CulpritJump { ..**c })
        .or_else(|| edges[from.id].last().map(|c| CulpritJump { ..*c }));
    (culprit, Some(message), Some(from.x1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixtures are ASCII art rather than inline JSON so a run of `#` never has
    /// to fight Rust's raw-string delimiters, and so the shape is scannable.
    fn level(name: &str, rows: &[&str], spawn: (i32, i32), entities: &[(&str, i32, i32)]) -> Level {
        let width = rows[0].len();
        assert!(
            rows.iter().all(|r| r.len() == width),
            "ragged fixture rows in {name}"
        );
        let tiles = rows
            .iter()
            .map(|r| format!("    \"{r}\""))
            .collect::<Vec<_>>()
            .join(",\n");
        let ents = entities
            .iter()
            .map(|(k, x, y)| format!("{{\"type\":\"{k}\",\"x\":{x},\"y\":{y}}}"))
            .collect::<Vec<_>>()
            .join(",");
        let src = format!(
            "{{\"format\":\"evo-platformer-level\",\"level_format_version\":1,\"name\":\"{name}\",
               \"width\":{width},\"height\":{},\"spawn\":{{\"x\":{},\"y\":{}}},
               \"tiles\":[\n{tiles}\n],\"entities\":[{ents}]}}",
            rows.len(),
            spawn.0,
            spawn.1
        );
        Level::from_json_str(&src).expect("fixture parses")
    }

    // Ground row 6, a two-tile pillar at x=6..8, shelf on top at row 4.
    const LADDER: &[&str] = &[
        "..........",
        "..........",
        "..........",
        "..........",
        "......###.",
        "......###.",
        "##########",
        "##########",
    ];

    // Ground row 6 at x 0..=5 and x 11..=19: a five-tile pit. Wider than the 4.00 a
    // running jump covers, and *inside* the 6.00 a sprinting one covers, which is what
    // makes it the fixture for the sprint: the same level is unbeatable and beatable
    // depending only on whether the oracle is allowed to sprint.
    const GAP: &[&str] = &[
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "######.....#########",
        "######.....#########",
    ];

    // The same shape with a seven-tile pit: past even a sprint jump, so the diagnostic
    // branch fires. Six tiles of runway is more than a sprint's 1.125, so the gap really
    // is the problem and the report must not blame the run-up.
    const GAP7: &[&str] = &[
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "######.......#######",
        "######.......#######",
    ];

    // A six-tile tower on a short ledge: its top is four tiles up, well past the 2.24 a
    // discrete full jump reaches, and the goal sits on top of it. Only a wall climb gets
    // there, so this fixture is the one that proves the oracle models wall jumps at all.
    const TOWER: &[&str] = &[
        "..........",
        "..........",
        "......###.",
        "......###.",
        "......###.",
        "......###.",
        "##########",
        "##########",
    ];

    #[test]
    fn platform_extraction_splits_only_at_real_breaks() {
        let lv = level("ladder", LADDER, (1, 5), &[("goal", 7, 3)]);
        let shape: Vec<(i32, i32, i32)> = extract_platforms(&lv)
            .iter()
            .map(|p| (p.x0, p.x1, p.surface_row))
            .collect();
        assert_eq!(shape, vec![(6, 8, 4), (0, 5, 6), (9, 9, 6)]);
    }

    #[test]
    fn a_held_jump_reaches_a_two_tile_shelf_that_a_tap_jump_cannot() {
        let lv = level("ladder", LADDER, (1, 5), &[("goal", 7, 3)]);
        let platforms = extract_platforms(&lv);
        // Take off far enough back that the body has cleared the pillar's rows by
        // the time it arrives: the shelf is 2 tiles up and the body is 0.9 tall,
        // so a full jump's 2.5-tile apex leaves well under a tile of clearance.
        let full = simulate_jump(
            &lv,
            &platforms,
            3,
            6,
            TakeoffDir::Right,
            JumpHold::Full,
            LaunchSpeed::Run,
        );
        let short = simulate_jump(
            &lv,
            &platforms,
            3,
            6,
            TakeoffDir::Right,
            JumpHold::Short,
            LaunchSpeed::Run,
        );
        assert_eq!(
            full.landed_on,
            Some(0),
            "a held jump should land on the shelf: {full:?}"
        );
        // The tap jump runs into the pillar's flank and drops back to the ground.
        // Note the tap jump is not *shorter* in ticks: a blocked trajectory is
        // truncated horizontally, not vertically, so airtime is not monotonic in
        // jump height. Only the landing spot is meaningful here.
        assert_eq!(
            short.landed_on,
            Some(1),
            "a tap jump should be blocked and fall back to the ground: {short:?}"
        );
        assert!(full.ticks <= MAX_TRAJECTORY_TICKS);
    }

    #[test]
    fn a_walk_off_the_edge_is_not_reported_as_a_landing_on_the_same_platform() {
        let lv = level("ladder", LADDER, (1, 5), &[("goal", 7, 3)]);
        let platforms = extract_platforms(&lv);
        let walk = simulate_jump(
            &lv,
            &platforms,
            5,
            6,
            TakeoffDir::Right,
            JumpHold::None,
            LaunchSpeed::Run,
        );
        assert_ne!(
            walk.landed_on,
            Some(1),
            "walking off must not count as landing back on the take-off platform: {walk:?}"
        );
    }

    #[test]
    fn the_ladder_fixture_is_beatable() {
        let report = report(&level("ladder", LADDER, (1, 5), &[("goal", 7, 3)]));
        assert!(report.goal_reached, "{:?}", report.diagnosis);
        assert_eq!(report.start, Some(1), "spawn should sit on the ground platform");
        assert_eq!(report.goal_platform, Some(0));
        assert!(report.culprit.is_none());
    }

    #[test]
    fn a_five_tile_gap_needs_a_sprint_and_says_so() {
        let lv = level("gap", GAP, (1, 5), &[("goal", 17, 5)]);

        // Sprinting is enough: five tiles against a 6.00-tile sprint jump, with six tiles
        // of runway to build it in.
        let with = report(&lv);
        assert!(
            with.goal_reached,
            "a five-tile gap is inside a sprint jump: {:?}",
            with.diagnosis
        );

        // Take the sprint away and the same level becomes unbeatable — which is the whole
        // point of the ability, and the reason a level can be *about* it.
        let without = report_with(
            &lv,
            ReachOptions {
                sprint: false,
                ..ReachOptions::default()
            },
        );
        assert!(!without.goal_reached);
        let d = without.diagnosis.expect("a diagnosis");
        assert!(d.contains("gap too wide to run"), "unexpected diagnosis: {d}");
        assert!(d.contains("6.00"), "the sprint distance belongs in the answer: {d}");
        assert!(
            without.culprit.is_some(),
            "the editor needs a jump to highlight"
        );
        assert_eq!(without.frontier_x, Some(5));
    }

    #[test]
    fn a_seven_tile_gap_is_reported_as_too_wide_for_even_a_sprint() {
        let report = report(&level("gap7", GAP7, (1, 5), &[("goal", 17, 5)]));
        assert!(!report.goal_reached);
        let d = report.diagnosis.expect("a diagnosis");
        assert!(d.contains("gap too wide"), "unexpected diagnosis: {d}");
        assert!(
            d.contains("even a sprinting"),
            "the answer should rule the sprint out on distance: {d}"
        );
        assert!(
            !d.contains("too wide to run"),
            "six tiles of runway is plenty for a sprint, so the run-up is not the problem: {d}"
        );
        assert_eq!(report.frontier_x, Some(5));
    }

    #[test]
    fn a_wall_too_tall_to_jump_is_climbed_by_wall_jumps() {
        let lv = level("tower", TOWER, (1, 5), &[("goal", 7, 1)]);

        // With wall climbing switched off, the tower is what it looks like: too tall. The
        // four-tile rise is past the 2.24 tiles a discrete full jump reaches.
        let without = report_with(
            &lv,
            ReachOptions {
                wall_climb: false,
                ..ReachOptions::default()
            },
        );
        assert!(!without.goal_reached);
        let d = without.diagnosis.clone().expect("a diagnosis");
        assert!(d.contains("wall too tall"), "unexpected diagnosis: {d}");
        assert_eq!(without.frontier_x, Some(5));

        // With it, the same wall is a staircase — and the goal, sitting on top of the
        // tower, is reached. Without this the oracle would call every wall-jump level
        // unbeatable, which is a lie that costs a designer their whole level.
        let with = report(&lv);
        assert!(
            with.goal_reached,
            "the tower should be climbable: {:?}",
            with.diagnosis
        );
        assert!(with.culprit.is_none());
    }

    #[test]
    fn a_scripted_wall_climb_gains_height_one_jump_at_a_time() {
        let lv = level("tower", TOWER, (1, 5), &[("goal", 7, 1)]);
        let platforms = extract_platforms(&lv);
        let top = platforms
            .iter()
            .find(|p| p.surface_row == 2 && p.contains_col(7))
            .expect("the tower top is a platform")
            .id;
        // Flush against the tower's face (its column is 6) on the ground platform.
        let climb = simulate_wall_climb(&lv, &platforms, 6.0 - PHYSICS.player_w, 6, TakeoffDir::Right);
        assert_eq!(
            climb.landed_on,
            Some(top),
            "the climb should end on the tower top: {climb:?}"
        );
        assert!(
            climb.ticks < MAX_CLIMB_TICKS,
            "a four-tile climb should not need the whole budget: {} ticks",
            climb.ticks
        );
        // And it really is a climb, not a leap: a single full jump from the same spot
        // cannot reach the top.
        let jump = simulate_jump(
            &lv,
            &platforms,
            5,
            6,
            TakeoffDir::Right,
            JumpHold::Full,
            LaunchSpeed::Sprint,
        );
        assert_ne!(
            jump.landed_on,
            Some(top),
            "the tower must be out of reach of a ground jump, or it is not a climb: {jump:?}"
        );
    }

    #[test]
    fn an_empty_platform_graph_still_spawns() {
        // A flat world with the goal directly overhead: still reachable is false,
        // and the report must not panic or invent a culprit.
        let flat = [".....", "..G..", "#####"];
        let report = report(&level("tiny", &flat, (1, 1), &[]));
        assert!(report.goal_reached, "{:?}", report.diagnosis);
    }
}
