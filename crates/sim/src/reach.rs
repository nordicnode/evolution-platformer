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
//!
//! **A spring pad is the third, and it is modelled rather than scripted.** A pad adds no move of
//! its own: it changes how far the move already under way goes, and it does that *inside a tick*.
//! So there is no canonical pad jump here — the oracle runs the sim's own pad rule at the same
//! point in the tick the episode loop runs it ([`crate::physics::spring_launch`]), and every
//! trajectory that passes through a pad's launch box is thrown by the same code that throws a
//! player. Modelling it late is what `REVIEW.md` §1.12 is: a level whose only way across a gap was
//! a pad at the lip was reported *unbeatable* while the population crossed it in a few generations,
//! because the trajectories were stepped one layer below the rule that owns the pad.

use serde::{Deserialize, Serialize};

use crate::level::{Level, Rect};
use crate::physics::{
    vine_exit_open, vine_mantle_anchor, AgentAction, Physics, PlayerState, TickContact, PHYSICS,
};

/// How long a single canonical jump trajectory may run.
pub const MAX_TRAJECTORY_TICKS: u32 = 240;

/// How long a canonical wall climb may run: ten seconds of wall time.
///
/// A climb is a *rhythm*, not a jump, so it needs a budget rather than a duration: at
/// roughly 2.4 tiles gained per 22-tick cycle it covers a 15-tile level's height in about
/// 150 ticks, and the cap is four times that so a tall shaft is not called unclimbable
/// because the oracle gave up early.
pub const MAX_CLIMB_TICKS: u32 = 600;

/// How long a canonical swim may run: twenty seconds of water time.
///
/// A swim is the slowest move in the game — the water caps a body at
/// [`Physics::water_speed`] (0.06 tiles a tick) — so the jump-sized budget of
/// [`MAX_TRAJECTORY_TICKS`] covers only fourteen tiles of pool, and a level with a forty-tile
/// lake was reported unreachable for no reason but the cap. Twenty seconds is 72 tiles at the
/// swim cap, which is longer than any shipped level, and it costs the sweep nothing on a
/// level with no water in it: the script is only run from a platform beside a pool.
pub const MAX_SWIM_TICKS: u32 = 1200;

/// Ticks one swim stroke is held, and how many of them release the jump.
///
/// A body swims while the button is down and sinks slowly while it is up, and a *fresh*
/// press at the surface is what breaches the pool's lip (see `PlayerState::breach`). A
/// swimmer therefore has to let go now and then — that is what mashing jump does — so the
/// script pulses: five ticks held, one released. The release is the shortest one that a
/// fresh press can follow, and five held ticks still move the body at the swim rate, so the
/// crossing loses about an eighth of a tile to each stroke and nothing else.
pub const SWIM_STROKE_TICKS: u32 = 6;
/// Ticks of a swim stroke during which the jump is held (`SWIM_STROKE_TICKS - 1`).
const SWIM_HOLD_TICKS: u32 = SWIM_STROKE_TICKS - 1;

/// Which abilities the oracle is allowed to use.
///
/// All three are on by default: the question the oracle answers is "can the game be beaten",
/// and the game has sprint, wall jumps and spring pads. Turning one off is how a level is checked
/// for *needing* it — a wall level should be unreachable without [`Self::wall_climb`], a five-tile
/// pit without [`Self::sprint`], and a level whose crossing is a pad at the lip without
/// [`Self::spring`] — and each of those claims is worth a test rather than a comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReachOptions {
    pub sprint: bool,
    pub wall_climb: bool,
    /// Whether a pad may throw a body.
    ///
    /// The third ability, and the only one with no script behind it: a pad changes how far a
    /// trajectory goes, so what it needed was the sim's own tick rule inside the trajectories the
    /// oracle already simulates (see the module note). Off, a level that needs a pad becomes
    /// unreachable — which is how the shape the owner reported is pinned as a *pad* level rather
    /// than merely a beatable one.
    pub spring: bool,
    /// Whether a body may swim across a pool.
    ///
    /// Water is not an obstacle, it is a medium: a body that walks in is caught by it, crosses
    /// at [`Physics::water_speed`] and breaches out of the far lip. A level whose only route is
    /// a wide pool was reported unreachable while the game crossed it without a jump, because a
    /// canonical *jump* falls through water and the oracle had no other move to try. Off, that
    /// level reads unreachable again — which is how the crossing is pinned as a swim rather than
    /// as a jump that happens to land.
    pub swim: bool,
    /// Whether a body may climb a vine.
    ///
    /// The vine is the one route in the game that needs no jump at all: a body that overlaps a
    /// vine while airborne and holds jump goes up it. With the move switched off, a level whose
    /// only way up is a hanging vine is unreachable — which is what the oracle used to answer
    /// about every one of them, and why no route graph could point a population at a vine.
    pub vine: bool,
}

impl Default for ReachOptions {
    fn default() -> Self {
        ReachOptions {
            sprint: true,
            wall_climb: true,
            spring: true,
            swim: true,
            vine: true,
        }
    }
}

/// Version of the **move set** the oracle sweeps: bump when a canonical move, its script, the
/// take-off grid, or a per-tick rule the trajectories run changes.
///
/// Deliberately *not* [`crate::SIM_VERSION`]. That constant's contract is physics, tile
/// semantics and episode termination, and the oracle moves whenever one of those does — but
/// the oracle also changes on its own. The take-off grid went from five sub-tile offsets to
/// twenty when L3's spike patch turned out to be clearable only from a band inside a column;
/// a wall climb's script, a new direction and the runway a sprint is credited with are the
/// same kind of change. None of them touches a physics constant, so none of them moves
/// `SIM_VERSION`, and every one of them changes what the oracle answers.
///
/// That is the whole reason this constant exists: the route cache beside a level file
/// (`crate::route_cache`) is keyed on both versions, because an answer is only worth keeping
/// while the *same oracle* would give it again. A cache that knew only the sim version would
/// serve a five-offset sweep's answer to a twenty-offset sweep, which is precisely the
/// false negative the offsets exist to prevent.
///
/// **Two since the oracle learned about spring pads**, which is the case that shows this guards a
/// *rule* and not only a script: a pad adds no canonical move, but it changes what the same take-off
/// reaches, and the answers beside a level file are keyed on this version — so every cached verdict
/// computed before pads were modelled is a miss rather than a stale "NOT beatable" about terrain the
/// population can cross.
/// v3: a take-off position whose body box overlaps solid ground is no longer swept
/// ([`takeoff_fits`]), so the move set lost moves that only existed because a body was *placed*
/// inside a step; and of the moves that reach one landing, the graph now keeps the one with the most
/// *margin* rather than the cheapest ([`better_plan`]). Both are changes to the set itself rather than
/// to a rule — the same terrain can now answer "not reachable" where it used to answer "reachable via
/// a jump from inside the wall", and a route now stores the take-off whose landing is further inside
/// the ledge — so cached verdicts and route graphs from v2 are misses.
/// v4: a tie between two plans of equal cost and margin is resolved by the sweep's own order again —
/// the first move found is kept, as [`better_plan`] has always said. A third term had been added,
/// breaking ties by take-off position, and it is the position a body is walked to that made it a
/// move-set change: preferring the later take-off left the replay unable to press from a stand's lip
/// (`l2`..`l5` stalled with the hop uncrossed), preferring the earlier one put `l9`'s first crossing at
/// an aim the four-tick cadence cannot stop on. No canonical move, script or grid changed, but which
/// of several swept take-offs an edge stores did, so the stamp moves. It is also why this one was
/// expensive to find: a v3 sidecar survived the first change, so the same terrain answered with two
/// different routes under one version — green on the machine holding the cache, red in CI, which
/// rebuilds the graph from the terrain.
/// v5: the oracle learned the game's two **media** — a swim across a pool and a climb up a
/// vine. Both are canonical scripts over the real stepper, like the wall climb, and both add
/// moves the jump sweep could not express: water is not solid, so a jump into it never lands,
/// and a vine is not a wall, so a climb finds nothing to press against. Every cached verdict and
/// route graph from v4 is a miss rather than a stale answer about terrain the game crosses.
/// v6: a **jump that catches a vine is no longer a jump**. The ballistic sweep used to hold the
/// jump button through a vine in its flight path and record the climb as a jump edge, which handed
/// the route graph a plan ("jump from three tiles to the left") whose landing only replays if the
/// body happens to grab the vine on the way. A vine move owns that landing; the jump sweep gives it
/// up. No script changed, but which plan an edge stores did — the same kind of change as v4, and
/// the same reason the cache keys on this constant rather than on the sim version.
/// v7: **a vine's own block is a route.** Holding the climb at the top of a vine whose run ends
/// under a one-tile-thick block now hauls the body onto that block — the physics' pull-up — so
/// the sweep's vine edges can land on a stand *above* the vine instead of only beside it. The
/// move set changed rather than the terrain: an old answer that called a vine under a lid a dead
/// end is a stale answer about a move the game now has, which is exactly what this constant is
/// for.
/// v8: **the flag's own stand is a stand a body can reach the flag from.** The graph took the
/// topmost stand sharing the flag's *column* as the flag's ground, which was true of every flag
/// standing on a floor and false of one near the top of a level: the floor seven tiles below
/// shares the column, so a body under a vine was told it was already home and the route walked it
/// to the flag's x instead of up the vine. No canonical move changed; the graph's own answer did,
/// and a cached route is an answer about exactly that, so the stamp moves with it.
/// v9: **a moving platform is a stand.** The verdict's table now carries both ends of every
/// shelf's travel, each at the deck's own (mid-cell) height, with a ride edge between them, so a
/// level whose only crossing is a lift or a slider is answered as beatable instead of as a gap
/// no jump can clear. No canonical move changed — the report gained two kinds of stand and an
/// edge the cycle makes true — but an answer about a level with a mover in it is a different
/// answer, so every cached verdict and route graph from v8 is a miss.
/// v10: **a moving platform is a stand, and its travel is a ride.** The route graph now carries both
/// ends of every shelf's travel as stands of its own, with a `ride` edge between them that the BFS
/// counts and no replay can press. The nodes moved and every hop count on a level with a mover in it
/// changed, which is exactly what a cached answer is keyed on — and the reason is not cosmetic: a
/// stall at a shuttle's ledge used to answer "the flag cannot be reached from here", so the stall
/// curriculum refused to derive a practice level and the run was left with nothing.
pub const MOVE_SET_VERSION: u32 = 10;

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

/// A standable surface: a maximal horizontal run of solid tiles with open space above it, or the
/// deck of a moving platform at one end of its travel.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Platform {
    pub id: usize,
    /// Inclusive tile column range of the surface.
    pub x0: i32,
    pub x1: i32,
    /// Tile row of the surface's cell. Its top edge is at world `y = surface_row` — except for a
    /// mover, whose deck is half a tile up inside the cell ([`Self::surface_y`]).
    pub surface_row: i32,
    /// The **top edge of the surface**, in world y. Ground: `surface_row as f32`. A moving
    /// platform: the deck's own top, which is mid-cell by [`crate::level::PlatformSpec`]'s design.
    ///
    /// A float because a shelf's deck is half a tile above the floor it is authored on, and the
    /// oracle's whole promise is that it only ever disagrees with the sim about *which* move is
    /// expressible — never about how far one goes. Rounding the deck to the nearest tile edge to
    /// keep this an integer would be a half-tile lie in every jump off it.
    pub surface_y: f32,
    /// For a moving platform's deck: the id of its deck at the **other end** of the travel. A
    /// rider can wait for the shelf and be carried between the two, so the report treats the pair
    /// as one edge of its own ([`ride`]). `None` for ground.
    pub ride: Option<usize>,
}

impl Platform {
    pub fn center_x(&self) -> f32 {
        (self.x0 + self.x1) as f32 * 0.5 + 0.5
    }

    /// World-space surface box, for drawing the validation overlay.
    pub fn surface_rect(&self) -> Rect {
        Rect::new(
            self.x0 as f32,
            self.surface_y,
            (self.x1 - self.x0 + 1) as f32,
            crate::physics::PHYSICS.platform_h,
        )
    }

    pub fn contains_col(&self, col: i32) -> bool {
        col >= self.x0 && col <= self.x1
    }
}

/// How long the jump button is held during a canonical trajectory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    pub fn with_runway(
        dir: TakeoffDir,
        hold: JumpHold,
        speed: LaunchSpeed,
        runway: f32,
        ph: &Physics,
    ) -> Takeoff {
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
    /// The `(column, surface_row)` it came to rest on, whenever [`Self::landed_on`] is
    /// set: *where* on the platform the body touched down, which is what the route
    /// graph ([`goal_routes`]) needs — a platform can hold several stands a body cannot
    /// walk between, and the landing column is what says which one a jump reached.
    pub landed_at: Option<(i32, i32)>,
    pub touched_goal: bool,
    pub hit_spike: bool,
    pub fell_out: bool,
    pub ticks: u32,
}

/// One candidate move out of a platform: the trajectory it produced, the take-off that names it,
/// and — when it is not a single jump at all — the climb script that made it.
///
/// The distinction has to be carried rather than inferred. A wall climb is pushed with the same
/// `CulpritJump` shape a jump has (`Full` hold, `Run` speed: what a climb asks of the buttons), but
/// it is a *rhythm* of repeated wall jumps that no single take-off reproduces, and a caller that
/// wanted to execute the move — the route graph's plan, and the replay that walks a route — would
/// press one jump and fall short.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveEdge {
    pub traj: Trajectory,
    pub jump: CulpritJump,
    /// The side the wall is on, when this move is a canonical climb rather than one jump.
    pub climb: Option<TakeoffDir>,
    /// The direction of travel and whether the entry was a dive, when this move is a swim.
    pub swim: Option<(TakeoffDir, bool)>,
    /// The exit side, when this move is a vine climb.
    pub vine: Option<TakeoffDir>,
}

/// A single canonical jump at a single spot: enough to name the culprit in the
/// editor when a level is unreachable.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

/// A verdict a level's own file has already been asked for: the answer, and the abilities it
/// was computed with.
///
/// The pair is the unit the cache stores, because a [`ReachReport`] alone does not say what
/// it is an answer *to*. A report computed with sprinting switched off is a different claim
/// about the same terrain, so a cache that could not tell the two apart would answer a
/// question nobody asked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CachedVerdict {
    pub options: ReachOptions,
    pub report: ReachReport,
}

/// Extract platforms from the tile grid (§9.1).
///
/// Ground counts as one long platform. Spikes are not platforms: they are lethal
/// and non-solid, so you can neither stand on one nor be carried by one.
///
/// **A spike does not cut the run in this table, and that asymmetry with the route graph is
/// deliberate.** The graph splits a spiked run at the spike (its stands come from
/// `landing_zone_bounds`) because a *plan* is made of landings; this table does not, because a spike
/// is not solid and a run of ground with spikes on it is still one surface the body can leave from
/// and land on. Splitting here was tried and reverted: `scratch-spike-tight` — a floor of spikes two
/// columns apart, which the population solves 11 times in 24 — came back **unbeatable**, because a
/// canonical trajectory holds its direction for the whole flight and so cannot take the one-tile pad
/// a policy takes by releasing mid-air. The verdict's move set is coarser than the body's, and where
/// it is coarser it must not invent a wall; the honest statement of that gap is the note on
/// [`simulate_jump_with`] and the deferred item in `FINDINGS.md`, not a false negative here.
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
                surface_y: row as f32,
                ride: None,
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

/// Every surface the oracle may stand on: the tile grid's own platforms, plus **both decks of
/// every moving platform**.
///
/// A mover is two stands, not one band. Its deck is a real box at two places a body can be —
/// parked at the near end and parked at the far end — and a body that can reach either can wait
/// for the cycle and be carried to the other ([`Platform::ride`]). Modelling the swept band as one
/// long surface instead would hand every take-off on it the runway of a fifteen-tile floor, which
/// is the sort of generous answer this oracle exists not to give.
///
/// The ids continue after the terrain's, so a report's own indices still address one table.
pub fn reach_platforms(level: &Level) -> Vec<Platform> {
    let mut out = extract_platforms(level);
    for spec in &level.platforms {
        let clear =
            crate::physics::clear_travel(spec.rect, spec.span, spec.vertical, spec.dir, level);
        let step = clear * f32::from(spec.dir);
        let mut far = spec.rect;
        if spec.vertical {
            far.y -= step;
        } else {
            far.x += step;
        }
        if clear <= 1e-4 {
            // A shelf with nowhere to go is a deck: one stand, no ride.
            let id = out.len();
            out.push(deck_platform(id, spec.rect, None));
            continue;
        }
        let near = out.len();
        let away = near + 1;
        out.push(deck_platform(near, spec.rect, Some(away)));
        out.push(deck_platform(away, far, Some(near)));
    }
    out
}

/// One end of a moving platform's travel, as a standable surface.
fn deck_platform(id: usize, rect: Rect, ride: Option<usize>) -> Platform {
    let x0 = rect.x.round() as i32;
    Platform {
        id,
        x0,
        x1: x0 + rect.w.round().max(1.0) as i32 - 1,
        surface_row: rect.y.floor() as i32,
        surface_y: rect.y,
        ride,
    }
}

/// The stand a body has just landed on by *crossing a deck's top*, if any.
///
/// A moving platform is not terrain, so the real stepper does not ground a body on it — the sim's
/// own landing rule lives in the episode loop. The oracle's canonical trajectories therefore have
/// to ask the question themselves, and they ask it the way the sim does: **a crossing**, feet at
/// or above the deck last tick and at or below it now, with the body over the deck's columns. A
/// position test instead would snap a body falling three tiles above a deck onto it.
fn deck_landing(
    platforms: &[Platform],
    prev_feet: f32,
    player: &PlayerState,
    ticks: u32,
) -> Option<Trajectory> {
    if player.vy < 0.0 {
        return None;
    }
    let feet = player.y + player.h;
    let (c0, c1) = crate::physics::tile_span(player.x, player.w);
    platforms
        .iter()
        .find(|p| {
            p.ride.is_some()
                && c0 <= p.x1
                && c1 >= p.x0
                && prev_feet <= p.surface_y + 1e-3
                && feet >= p.surface_y - 1e-3
        })
        .map(|p| Trajectory {
            landed_on: Some(p.id),
            landed_at: Some((c0.max(p.x0), p.surface_row)),
            touched_goal: false,
            hit_spike: false,
            fell_out: false,
            ticks,
        })
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
///
/// **Public because a replayed move is the same move.** `evo::route_play` drives a body along these
/// very trajectories through the episode loop, so it presses the plan with this function rather
/// than with a second reading of what a held jump means; the two could otherwise disagree about a
/// move the oracle had already proved.
pub fn trajectory_action(takeoff: &Takeoff, hovering: bool, t: u32, ph: &Physics) -> AgentAction {
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
        surface_row as f32,
        &Takeoff::full(dir, hold, speed, &PHYSICS),
    )
}

/// One tick of a canonical trajectory: the real stepper, and the sim's own per-tick rules.
///
/// The oracle's promise is that it can only ever disagree with the sim about whether a *move* is
/// expressible in the action space — never about how far a move goes. Anything the episode loop does
/// to a body between two decisions therefore belongs in here, which is why the spring rule is the
/// sim's own call at the sim's own point in the tick rather than a second statement of it: a
/// trajectory that passes through a pad's launch box is thrown by the same code, to the same
/// velocity, as a player.
///
/// The launch is returned as well as the contact, because a pad can throw a body on the very tick it
/// touches down — the tick then ends *airborne*, and the tracer must not read that touchdown as a
/// landing.
fn oracle_tick(
    level: &Level,
    player: &mut PlayerState,
    action: AgentAction,
    options: ReachOptions,
) -> (TickContact, bool) {
    let contact = player.tick(level, &PHYSICS, action);
    let launched = options.spring && crate::physics::spring_launch(level, player, &PHYSICS);
    (contact, launched)
}

/// As [`simulate_jump`], but with the body's left edge placed at an exact x and the exact
/// horizontal speed it carries into the take-off, so callers can sweep both the sub-tile
/// positions [`TAKEOFF_OFFSETS`] covers and the runway-dependent launch speeds.
///
/// This is the oracle at full strength — every ability the game has, pads included. The sweep
/// switches abilities off in one place ([`edges_at_column`]), which is why this entry point has no
/// options of its own.
pub fn simulate_jump_at(
    level: &Level,
    platforms: &[Platform],
    takeoff_x: f32,
    surface_y: f32,
    takeoff: &Takeoff,
) -> Trajectory {
    simulate_jump_with(
        level,
        platforms,
        takeoff_x,
        surface_y,
        takeoff,
        ReachOptions::default(),
    )
}

/// [`simulate_jump_at`] with the oracle's abilities in the caller's hands.
fn simulate_jump_with(
    level: &Level,
    platforms: &[Platform],
    takeoff_x: f32,
    surface_y: f32,
    takeoff: &Takeoff,
    options: ReachOptions,
) -> Trajectory {
    let ph = &PHYSICS;
    let mut player = PlayerState::spawn(level, ph);
    player.w = ph.player_w;
    player.h = ph.player_h;
    player.x = takeoff_x;
    player.y = surface_y - ph.player_h;
    player.vx = takeoff.launch_vx;
    // The cap has to come along with the speed: a body launched at sprint speed with a
    // running cap would have its launch clamped away on the very first tick, and the
    // oracle would report the sprint jump as a running one.
    player.set_speed_cap(takeoff.launch_vx.abs().max(ph.run_speed));
    player.vy = 0.0;
    // The body is authored standing on `surface_row`, which is by construction a
    // surface tile, so a jump is legal on tick zero.
    player.grounded = true;
    if let Some(death) = hazard_start(level, &player) {
        return death;
    }

    let mut hover = takeoff.hold != JumpHold::None;
    // A "landing" only counts once the body has actually left the ground, so
    // walking off an edge is not misreported as landing on the take-off platform
    // during the ticks it takes to clear the edge.
    let mut airborne = false;
    let mut prev_feet = player.y + player.h;
    for t in 0..MAX_TRAJECTORY_TICKS {
        let action = trajectory_action(takeoff, hover, t, ph);
        let (contact, launched) = oracle_tick(level, &mut player, action, options);
        let feet = player.y + player.h;
        // A pad's throw is not a button, so there is nothing left to hold: `bounce` spends the jump
        // cut, and a trajectory that went on claiming a held jump would be asking for an action the
        // body has no reason to be making.
        if launched || player.vy >= 0.0 {
            hover = false;
        }
        if !player.grounded {
            airborne = true;
        }
        if let Some(end) = trajectory_terminal(level, &player, &contact, t + 1) {
            return end;
        }
        // **A jump that catches a vine is not a jump.** The sim will happily hold the button
        // through a vine in the flight path and climb it — true of the game, and it is why a vine
        // in a jump's arc used to appear as a jump edge. But a route plan has to name the move the
        // body makes: a jump at a take-off a tile to the left of the vine cannot be replayed by
        // standing under the vine, and a plan that says "jump" while the body climbs is a plan the
        // replay can only stumble into. The vine script ([`simulate_vine_climb`]) owns that move,
        // so the ballistic sweep gives its landing up to it.
        if player.on_vine {
            return Trajectory {
                landed_on: None,
                landed_at: None,
                touched_goal: false,
                hit_spike: false,
                fell_out: false,
                ticks: t + 1,
            };
        }
        if airborne && !launched {
            if let Some(deck) = deck_landing(platforms, prev_feet, &player, t + 1) {
                return deck;
            }
        }
        prev_feet = feet;
        if contact.landed && airborne && !launched {
            // Landing back on the take-off platform with no forward progress is
            // not an edge; it is reported as "landed here" and filtered by BFS.
            return landed_trajectory(level, platforms, &player, t + 1);
        }
    }
    Trajectory {
        landed_on: None,
        landed_at: None,
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
/// The recorded column is the one the body is *supported* by: the first column it spans
/// that is ground it could stand on. A body cannot come to rest with a spike at its head
/// or a wall in its torso — those ends are terminal before the landing is read — so the
/// fallback to the first solid column is belt and braces; both `landed_on` and
/// `landed_at` are read off the same column, so they can never disagree about where the
/// body is.
fn landed_trajectory(
    level: &Level,
    platforms: &[Platform],
    player: &PlayerState,
    ticks: u32,
) -> Trajectory {
    let row = (player.y + player.h).floor() as i32;
    let (c0, c1) = crate::physics::tile_span(player.x, player.w);
    let support = (c0..=c1)
        .find(|c| platform_at(platforms, row, *c).is_some() && level.is_standable(*c, row))
        .or_else(|| (c0..=c1).find(|c| platform_at(platforms, row, *c).is_some()));
    Trajectory {
        landed_on: support.and_then(|c| platform_at(platforms, row, c)),
        landed_at: support.map(|c| (c, row)),
        touched_goal: false,
        hit_spike: false,
        fell_out: false,
        ticks,
    }
}

/// A body cannot *begin* a move inside a hazard.
///
/// The game kills a body the tick it overlaps a spike, but the hazard check runs after
/// movement — so a take-off authored standing in a spike would leave the tile between
/// tick zero and tick one and never be checked at all. That is not a legal move, it is a
/// body that was already dead: to stand at that x the body had to walk into the spike
/// first. Sweeping take-off positions over a spike-topped column let `report` invent an
/// edge out of exactly such a spot and call a sealed tower beatable (finding 52), so the
/// sweep's answer is now the game's: a death, and no edge.
fn hazard_start(level: &Level, player: &PlayerState) -> Option<Trajectory> {
    let (hx0, hx1) = crate::physics::tile_span(player.x, player.w);
    let (hy0, hy1) = crate::physics::row_span(player.y, player.h);
    let body = player.aabb();
    // The sim's own hazard box, not the tile: a spike's drawn sprite leaves the top
    // [`crate::physics::SPIKE_TOP_CLEARANCE`] of its cell transparent, and a body standing
    // only in that band is a legal take-off. Asking the oracle a *stricter* question than
    // the game asks would be the false negative the sweep exists to avoid.
    let inside = (hy0..=hy1).any(|ty| {
        (hx0..=hx1).any(|tx| {
            (level.is_spike(tx, ty)
                && body.overlaps(&crate::physics::spike_lethal_box(tx, ty)))
                // Lava is the whole cell, so the sweep asks the same question the sim does.
                || (level.is_lava(tx, ty)
                    && body.overlaps(&crate::level::Rect::new(tx as f32, ty as f32, 1.0, 1.0)))
        })
    });
    inside.then_some(Trajectory {
        landed_on: None,
        landed_at: None,
        touched_goal: false,
        hit_spike: true,
        fell_out: false,
        ticks: 0,
    })
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
    if contact.touched_spike || contact.touched_lava {
        return Some(Trajectory {
            landed_on: None,
            landed_at: None,
            touched_goal: false,
            hit_spike: true,
            fell_out: false,
            ticks,
        });
    }
    if level.goals.iter().any(|g| player.aabb().overlaps(g)) {
        return Some(Trajectory {
            landed_on: None,
            landed_at: None,
            touched_goal: true,
            hit_spike: false,
            fell_out: false,
            ticks,
        });
    }
    if player.y > level.height as f32 {
        return Some(Trajectory {
            landed_on: None,
            landed_at: None,
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
    simulate_wall_climb_with(
        level,
        platforms,
        start_x,
        surface_row,
        dir,
        ReachOptions::default(),
    )
}

/// [`simulate_wall_climb`] with the oracle's abilities in the caller's hands.
///
/// A climb can meet a pad like any other trajectory can — a shaft with a pad on its floor is a
/// shaft whose first landing throws the body — so this runs the same tick as the jump does rather
/// than a second script that could drift from it.
fn simulate_wall_climb_with(
    level: &Level,
    platforms: &[Platform],
    start_x: f32,
    surface_row: i32,
    dir: TakeoffDir,
    options: ReachOptions,
) -> Trajectory {
    let ph = &PHYSICS;
    let mut player = PlayerState::spawn(level, ph);
    player.w = ph.player_w;
    player.h = ph.player_h;
    player.x = start_x;
    player.y = surface_row as f32 - ph.player_h;
    // The caller chose this spot because the platform is under it, so the body starts
    // standing — exactly as `simulate_jump_at` does for a take-off.
    player.grounded = true;
    if let Some(death) = hazard_start(level, &player) {
        return death;
    }

    let mut airborne = false;
    for t in 0..MAX_CLIMB_TICKS {
        let action = climb_action(dir, t, ph);
        let (contact, launched) = oracle_tick(level, &mut player, action, options);
        if !player.grounded {
            airborne = true;
        }
        if let Some(end) = trajectory_terminal(level, &player, &contact, t + 1) {
            return end;
        }
        if contact.landed && airborne && !launched {
            return landed_trajectory(level, platforms, &player, t + 1);
        }
    }
    Trajectory {
        landed_on: None,
        landed_at: None,
        touched_goal: false,
        hit_spike: false,
        fell_out: false,
        ticks: MAX_CLIMB_TICKS,
    }
}

/// One canonical wall climb's rhythm: the two arms and how long the jump is held.
///
/// Hold through the ascent, then release for two ticks. The pause is what makes every press a
/// *fresh* one, which is what an edge-triggered jump needs, and it is short enough that the body
/// has barely started sliding again when the next wall jump fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClimbScript {
    /// The arm held through the ascent: the jump, in the climb's direction.
    pub jump: AgentAction,
    /// The arm held during the release: the direction, with the jump let go.
    pub press: AgentAction,
    /// Decision blocks the jump is held for.
    pub hold: u32,
}

/// The climb script for a wall on `dir`'s side.
///
/// Derived from the physics rather than declared, because the interval is `air_time / 2`: a climb
/// re-jumps as the body passes its own apex, and a cadence written down by hand would be a climb
/// that stops working the day gravity changes.
pub fn climb_script(dir: TakeoffDir, ph: &Physics) -> ClimbScript {
    let (jump, press) = match dir {
        TakeoffDir::Left => (AgentAction::RetreatJump, AgentAction::Retreat),
        TakeoffDir::Right => (AgentAction::RunJump, AgentAction::Run),
        TakeoffDir::Still => (AgentAction::JumpInPlace, AgentAction::Walk),
    };
    ClimbScript {
        jump,
        press,
        hold: (ph.air_time_ticks() * 0.5).ceil() as u32,
    }
}

/// The tick-by-tick action of a climb against a wall on `dir`'s side: [`climb_script`]'s rhythm at
/// tick `t` since the climb started.
///
/// Beside [`trajectory_action`] for the same reason it is public: the oracle's climb and a replayed
/// climb have to be the same button presses, so there is exactly one statement of them.
pub fn climb_action(dir: TakeoffDir, t: u32, ph: &Physics) -> AgentAction {
    let script = climb_script(dir, ph);
    if t % (script.hold + 2) < script.hold {
        script.jump
    } else {
        script.press
    }
}

/// The arm for a direction and a jump, as every canonical script names one.
///
/// One function so the swim, the vine and the jump sweep cannot disagree about which action
/// "right, holding jump" is — the pick is the same one [`trajectory_action`] makes, and a gear
/// added to [`AgentAction`] has one place to be read.
fn arm(dir: TakeoffDir, jump: bool) -> AgentAction {
    match (dir, jump) {
        (TakeoffDir::Right, true) => AgentAction::SprintJump,
        (TakeoffDir::Right, false) => AgentAction::Sprint,
        (TakeoffDir::Left, true) => AgentAction::RetreatSprintJump,
        (TakeoffDir::Left, false) => AgentAction::RetreatSprint,
        (TakeoffDir::Still, true) => AgentAction::JumpInPlace,
        (TakeoffDir::Still, false) => AgentAction::Walk,
    }
}

/// Where a body can **enter the water** from `p`: a take-off position, and the side the pool
/// is on.
///
/// Only the platform's two ends are swept — water is something a body walks, falls or dives
/// *off* an edge into — and the test is the body's leading edge: a pool one column past the
/// ledge is a walk-off, a pool four columns past it is a dive, and a pool further than that is
/// not this level's crossing (a jump over it is the jump sweep's). The scan looks a few rows
/// down as well as across, because a canal is usually *below* the bank it is entered from.
fn swim_starts(level: &Level, p: &Platform) -> Vec<(f32, TakeoffDir)> {
    let mut out = Vec::new();
    for (col, dir) in [(p.x1, TakeoffDir::Right), (p.x0, TakeoffDir::Left)] {
        for &offset in &TAKEOFF_OFFSETS {
            let x = col as f32 + offset;
            if !takeoff_fits(level, x, p.surface_row as f32, &PHYSICS) {
                continue;
            }
            let lead = match dir {
                TakeoffDir::Right => x + PHYSICS.player_w,
                TakeoffDir::Left => x,
                TakeoffDir::Still => continue,
            };
            if water_ahead(level, lead, p.surface_row, dir) {
                out.push((x, dir));
            }
        }
    }
    out
}

/// Is there water at or below the body's leading edge, within a walk or a dive of it?
///
/// Four columns: a running jump reaches four tiles, and a body that leaves the bank with the
/// direction held is a body that will be over the pool. Three rows down: a canal under a raised
/// bank is still the pool the body falls into.
fn water_ahead(level: &Level, lead_x: f32, surface_row: i32, dir: TakeoffDir) -> bool {
    let sign = dir.sign() as i32;
    let col = lead_x.floor() as i32;
    (0..=4).any(|step| (-1..=3).any(|dy| level.is_water(col + sign * step, surface_row + dy)))
}

/// The tick-by-tick action of a canonical **swim**, as both the oracle and a replayed route
/// issue it.
///
/// An approach while the body is in the air, then a stroke pulse once it is in the water: the
/// button held for [`SWIM_HOLD_TICKS`] of every [`SWIM_STROKE_TICKS`] so a fresh press reaches
/// the surface now and then (a held press is never a new one, and only a new one breaches a
/// pool's lip). `dive` is whether the entry was a jump — which is the whole difference between
/// walking into a canal and throwing the body across the gap before it.
pub fn swim_action(dir: TakeoffDir, dive: bool, in_water: bool, t: u32) -> AgentAction {
    let jump = if in_water {
        t % SWIM_STROKE_TICKS < SWIM_HOLD_TICKS
    } else {
        dive
    };
    // **A run, never a sprint.** The sprint arms exist to build a take-off speed; a swimmer's
    // gear does not matter to the water (it clamps the body to `water_speed`), and on the release
    // tick — the one tick a stroke spends at the surface — a sprint arm would accelerate the
    // body *out* of the pool's cap and skate it across the remaining tiles at sprint speed. That
    // is not a stroke, it is a boat, and it crossed a pool in the test's `a_swim` fixture that a
    // swim should take twice as long over.
    match (dir, jump) {
        (TakeoffDir::Left, true) => AgentAction::RetreatJump,
        (TakeoffDir::Left, false) => AgentAction::Retreat,
        (TakeoffDir::Still, true) => AgentAction::JumpInPlace,
        (TakeoffDir::Still, false) => AgentAction::Walk,
        (TakeoffDir::Right, true) => AgentAction::RunJump,
        (TakeoffDir::Right, false) => AgentAction::Run,
    }
}

/// Simulate one canonical swim with the real physics stepper.
pub fn simulate_swim(
    level: &Level,
    platforms: &[Platform],
    takeoff_x: f32,
    surface_row: i32,
    dir: TakeoffDir,
    dive: bool,
) -> Trajectory {
    simulate_swim_with(
        level,
        platforms,
        takeoff_x,
        surface_row,
        dir,
        dive,
        ReachOptions::default(),
    )
}

/// [`simulate_swim`] with the oracle's abilities in the caller's hands.
///
/// The body starts standing at the lip with the run speed the platform's runway allows, so a
/// dive that needs a run-up has it and a pool beside a two-tile ledge is entered at a walk. From
/// there the script is [`swim_action`]'s, and every tick is the sim's own — water drag, the sink,
/// the stroke, the breach and the force of the pool's far wall are all the game's, not a second
/// statement of them. The trajectory ends where any other does: on a platform, at the flag, on a
/// hazard, in the void, or at [`MAX_SWIM_TICKS`].
fn simulate_swim_with(
    level: &Level,
    platforms: &[Platform],
    takeoff_x: f32,
    surface_row: i32,
    dir: TakeoffDir,
    dive: bool,
    options: ReachOptions,
) -> Trajectory {
    let ph = &PHYSICS;
    let mut player = PlayerState::spawn(level, ph);
    player.w = ph.player_w;
    player.h = ph.player_h;
    player.x = takeoff_x;
    player.y = surface_row as f32 - ph.player_h;
    // The body leaves the bank with the speed it could have built behind it: the swim itself is
    // capped at the water's rate, but the dive across a gap is a jump and needs the run-up.
    player.vx = dir.sign() * LaunchSpeed::Run.achievable(ph, 4.0);
    player.set_speed_cap(player.vx.abs().max(ph.run_speed));
    player.vy = 0.0;
    player.grounded = true;
    if let Some(death) = hazard_start(level, &player) {
        return death;
    }

    let mut airborne = false;
    let mut entered_water = false;
    for t in 0..MAX_SWIM_TICKS {
        let action = swim_action(dir, dive, player.in_water, t);
        let (contact, launched) = oracle_tick(level, &mut player, action, options);
        if player.in_water {
            entered_water = true;
        }
        if !player.grounded {
            airborne = true;
        }
        if let Some(end) = trajectory_terminal(level, &player, &contact, t + 1) {
            return end;
        }
        // Only a landing **after the water** is this move's landing. A body that walks off the
        // lip and touches down again on the bank — or jumps clean over the pool — is not a swim,
        // and the jump sweep already has that edge; counting it here would credit the water for a
        // crossing that never got wet.
        if contact.landed && airborne && !launched && entered_water {
            return landed_trajectory(level, platforms, &player, t + 1);
        }
    }
    Trajectory {
        landed_on: None,
        landed_at: None,
        touched_goal: false,
        hit_spike: false,
        fell_out: false,
        ticks: MAX_SWIM_TICKS,
    }
}

/// The contiguous run of vine cells a body standing on `p` can reach from `(col, surface_row)`:
/// `(bottom, top)` rows, when there is one.
///
/// The lowest cell has to be within a standing jump of the body's head — the body jumps from
/// rest and grabs the first vine it overlaps on the way up — and the run is the **contiguous**
/// shaft above it, because a body hanging on a vine does not pass a gap: it leaves the vine, and
/// the climb is over.
pub fn vine_run(level: &Level, col: i32, surface_row: i32, ph: &Physics) -> Option<(i32, i32)> {
    let reach = (ph.player_h + ph.discrete_max_jump_height()).ceil() as i32 + 1;
    for dy in 1..=reach {
        let row = surface_row - dy;
        if !level.is_vine(col, row) {
            continue;
        }
        let mut top = row;
        while top > 0 && level.is_vine(col, top - 1) {
            top -= 1;
        }
        return Some((row, top));
    }
    None
}

/// Where a body can **grab a vine** from `p`: a take-off position under a reachable vine, and the
/// top of that vine's run (what the exit is timed against).
fn vine_starts(level: &Level, p: &Platform) -> Vec<f32> {
    let mut out = Vec::new();
    for col in p.x0..=p.x1 {
        let x = col as f32 + (1.0 - PHYSICS.player_w) * 0.5;
        if !takeoff_fits(level, x, p.surface_y, &PHYSICS) {
            continue;
        }
        if let Some((_, top)) = vine_run(level, col, p.surface_row, &PHYSICS) {
            // Water wins the medium, so a vine in a flooded shaft is not a climb the game has.
            let flooded = (top..=p.surface_row).any(|row| level.is_water(col, row));
            if !flooded {
                out.push(x);
            }
        }
    }
    out
}

/// The tick-by-tick action of a canonical **vine climb**, as both the oracle and a replayed
/// route issue it.
///
/// Three phases, and the middle one is the whole mechanic: jump to reach the vine, hold jump
/// with **no direction** so the body climbs it (a direction would walk the body off the vine),
/// and press the exit side once its feet are level with the vine's top — which is the height of
/// the ledge the vine hangs beside. State rather than a tick count, because a replayed body does
/// not stand on exactly the swept pixel.
///
/// `exit` is [`TakeoffDir::Still`] for a vine that hangs under a **block** rather than beside a
/// ledge: there is nothing to press off, so the arm never changes — the climb is held to the top
/// and the physics' own pull-up carries the body over the block ([`vine_mantle_anchor`]). The
/// script is still the same three phases read as two, which is why this is one function.
///
/// `feet` is the body's bottom edge; `on_vine` is the sim's own reading of the medium.
pub fn vine_action(
    exit: TakeoffDir,
    on_vine: bool,
    feet: f32,
    top_row: i32,
    exiting: &mut bool,
) -> AgentAction {
    if *exiting {
        return arm(exit, true);
    }
    // The same window the physics holds the body in ([`crate::physics::vine_exit_open`]): the
    // press has to happen while the vine still lets go, and no earlier.
    if on_vine && vine_exit_open(feet, top_row) {
        *exiting = true;
        return arm(exit, true);
    }
    // Climb straight up, with the jump held. On the way up from the ground this is also the
    // jump that catches the vine, which is why there is no separate grab phase.
    AgentAction::JumpInPlace
}

/// Simulate one canonical vine climb with the real physics stepper.
///
/// `exit` is the side the body leaves toward, or [`TakeoffDir::Still`] for the pull-up over the
/// block the vine hangs from — see [`vine_action`] and [`vine_mantle_anchor`].
pub fn simulate_vine_climb(
    level: &Level,
    platforms: &[Platform],
    start_x: f32,
    surface_row: i32,
    exit: TakeoffDir,
) -> Trajectory {
    simulate_vine_climb_with(
        level,
        platforms,
        start_x,
        surface_row,
        exit,
        ReachOptions::default(),
    )
}

/// [`simulate_vine_climb`] with the oracle's abilities in the caller's hands.
fn simulate_vine_climb_with(
    level: &Level,
    platforms: &[Platform],
    start_x: f32,
    surface_row: i32,
    exit: TakeoffDir,
    options: ReachOptions,
) -> Trajectory {
    let ph = &PHYSICS;
    // The vine's top is a property of the level, so the caller names the position and this
    // function reads the run — one implementation of "where does this climb end" for the
    // sweep, the replay and the press check.
    let col = Level::tile_x(start_x + ph.player_w * 0.5);
    let top_row = match vine_run(level, col, surface_row, ph) {
        Some((_, top)) => top,
        // Nothing to grab: the move is a jump that lands where it started, which the graph
        // drops. Reporting it as a death would blame the level for the sweep's mistake.
        None => {
            return Trajectory {
                landed_on: None,
                landed_at: None,
                touched_goal: false,
                hit_spike: false,
                fell_out: false,
                ticks: 0,
            }
        }
    };
    let mut player = PlayerState::spawn(level, ph);
    player.w = ph.player_w;
    player.h = ph.player_h;
    player.x = start_x;
    player.y = surface_row as f32 - ph.player_h;
    player.grounded = true;
    if let Some(death) = hazard_start(level, &player) {
        return death;
    }

    let mut airborne = false;
    let mut exiting = false;
    for t in 0..MAX_CLIMB_TICKS {
        let feet = player.y + player.h;
        let action = vine_action(exit, player.on_vine, feet, top_row, &mut exiting);
        let (contact, launched) = oracle_tick(level, &mut player, action, options);
        if !player.grounded {
            airborne = true;
        }
        if let Some(end) = trajectory_terminal(level, &player, &contact, t + 1) {
            return end;
        }
        if contact.landed && airborne && !launched {
            return landed_trajectory(level, platforms, &player, t + 1);
        }
    }
    Trajectory {
        landed_on: None,
        landed_at: None,
        touched_goal: false,
        hit_spike: false,
        fell_out: false,
        ticks: MAX_CLIMB_TICKS,
    }
}

/// Whether a body standing with its left edge at `x` and its feet on `surface_row` fits in the
/// level's open space.
///
/// The oracle places bodies at exact positions instead of walking them there, so this is the one
/// question it has to ask *for* the body: does the box overlap a solid tile? The box is the body's
/// own ([`Physics::player_w`] × [`Physics::player_h`]), its feet sit on the surface, and the shrink
/// is one thousandth of a tile so that a body flush against a wall — which is exactly how
/// [`wall_climb_starts`] places one — counts as fitting rather than as overlapping by a rounding
/// error.
fn takeoff_fits(level: &Level, x: f32, surface_y: f32, ph: &Physics) -> bool {
    const SLACK: f32 = 1e-3;
    let (c0, c1) = crate::physics::tile_span(x + SLACK, ph.player_w - 2.0 * SLACK);
    let top = surface_y - ph.player_h + SLACK;
    let bottom = surface_y - SLACK;
    let (r0, r1) = (
        Level::tile_y(top),
        Level::tile_y(bottom - SLACK).max(Level::tile_y(top)),
    );
    for tx in c0..=c1 {
        for ty in r0..=r1 {
            if level.is_solid(tx, ty) {
                return false;
            }
        }
    }
    true
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

/// How many threads a sweep of one platform may use: every core but one.
///
/// The same trade `app::lab::training_threads` makes, for the same reason: a window that is
/// drawing while the oracle runs needs a core of its own, and one core is a cheap price for a
/// view whose whole purpose is to be watched while it works. A one-core machine still gets
/// one thread — a sweep that will not run is worse than a sweep that is slow.
fn sweep_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .saturating_sub(1)
        .max(1)
}

/// The fewest columns worth handing to another thread.
///
/// A column is `TAKEOFF_OFFSETS × directions × holds × speeds` jumps — a fifth of a
/// millisecond — so a platform narrower than this costs less to sweep in place than to split,
/// and a spawn is most of the price of splitting. Below the threshold nothing is spawned.
const MIN_PARALLEL_COLUMNS: usize = 8;

/// Run `f` over `items` on at most `threads` threads and return the results **in item order**.
///
/// Order is not a nicety here. A sweep's output is a *sequence* the report reads — the last
/// jump out of a platform is the jump a stuck run is blamed on
/// ([`report_with`]), and the route graph's crossing lists are built in it — so the chunks are
/// assembled by index and the answer is bit-identical to the sequential one. That equality is
/// asserted rather than assumed
/// (`a_parallel_sweep_answers_exactly_what_a_sequential_one_does`), because a scheduler is the
/// last thing a determinism guarantee should be quietly resting on.
fn sweep_map<T, R, F>(items: &[T], threads: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let threads = threads.min(items.len());
    if threads < 2 || items.len() < MIN_PARALLEL_COLUMNS {
        return items.iter().map(&f).collect();
    }
    // Small chunks dealt round-robin, not one slice per thread. Columns are not equal work — a
    // take-off beside a pit runs two hundred ticks where one over flat ground runs six — so an
    // even split would leave one thread sweeping the whole interesting end of the platform while
    // the rest waited.
    const CHUNK: usize = 4;
    let n_chunks = items.len().div_ceil(CHUNK);
    let mut per_thread: Vec<Vec<(usize, Vec<R>)>> = (0..threads).map(|_| Vec::new()).collect();
    std::thread::scope(|scope| {
        for (t, out) in per_thread.iter_mut().enumerate() {
            let f = &f;
            scope.spawn(move || {
                for i in (t..n_chunks).step_by(threads) {
                    let (start, end) = (i * CHUNK, ((i + 1) * CHUNK).min(items.len()));
                    out.push((i, items[start..end].iter().map(f).collect()));
                }
            });
        }
    });
    // Assembled by chunk index, which is what makes the answer the sequential one.
    let mut ordered: Vec<Option<Vec<R>>> = (0..n_chunks).map(|_| None).collect();
    for out in per_thread {
        for (i, chunk) in out {
            ordered[i] = Some(chunk);
        }
    }
    ordered.into_iter().flatten().flatten().collect()
}

/// Every canonical move out of every platform in `takeoffs` (§9.3): jumps at every sub-tile
/// take-off point and launch speed, walk-offs at the ends, and wall climbs at any end that has
/// a wall.
///
/// One edge list per take-off platform, in platform order, each in its own column order.
/// `landing` is the platform table a trajectory can come to rest on, which the caller decides:
/// [`report_with`] sweeps the level's own platforms onto themselves, while [`goal_routes`]
/// sweeps each *stand* (a landing zone, which a spike can split a platform into) as its own
/// take-off platform.
///
/// **The whole job is swept in parallel at once, and the pieces are assembled in order**
/// ([`sweep_map`]). The take-off grid is `columns × TAKEOFF_OFFSETS × directions × holds ×
/// speeds` — twelve thousand jumps for a sixty-tile floor, and this sweep is essentially the
/// whole cost of a level's route graph and of the editor's verdict — so the parallelism has to
/// reach across the job rather than inside one platform: `l1-flat-run` is a single sixty-column
/// platform (sweeping platforms one at a time in parallel would leave it as slow as it is), and
/// a level of narrow ledges is the opposite way round. Flattened, both are as wide as the
/// level.
fn edges_from_all(
    level: &Level,
    landing: &[Platform],
    takeoffs: &[Platform],
    options: &ReachOptions,
) -> Vec<Vec<MoveEdge>> {
    edges_from_all_with(level, landing, takeoffs, options, sweep_threads())
}

/// As [`edges_from_all`], with the sweep's thread count in the caller's hands: `1` is the plain
/// sequential sweep, and the answer is the same for every count — which is what
/// `a_parallel_sweep_answers_exactly_what_a_sequential_one_does` is there to keep true.
fn edges_from_all_with(
    level: &Level,
    landing: &[Platform],
    takeoffs: &[Platform],
    options: &ReachOptions,
    threads: usize,
) -> Vec<Vec<MoveEdge>> {
    // The job list, in platform-then-column order, so the flat results come back in the order
    // the per-platform lists need to be cut from them.
    let mut job: Vec<(usize, i32)> = Vec::new();
    let mut widths: Vec<usize> = Vec::with_capacity(takeoffs.len());
    for (i, p) in takeoffs.iter().enumerate() {
        for col in p.x0..=p.x1 {
            job.push((i, col));
        }
        widths.push((p.x1 - p.x0 + 1).max(0) as usize);
    }

    let swept: Vec<Vec<MoveEdge>> = sweep_map(&job, threads, |(i, col)| {
        edges_at_column(level, landing, &takeoffs[*i], options, *col)
    });

    let mut out: Vec<Vec<MoveEdge>> = Vec::with_capacity(takeoffs.len());
    let mut cursor = 0;
    for (i, p) in takeoffs.iter().enumerate() {
        let mut edges: Vec<MoveEdge> = swept[cursor..cursor + widths[i]].concat();
        cursor += widths[i];
        if options.wall_climb {
            let ph = &PHYSICS;
            for (start_x, dir) in wall_climb_starts(level, p, ph) {
                let traj =
                    simulate_wall_climb_with(level, landing, start_x, p.surface_row, dir, *options);
                edges.push(MoveEdge {
                    traj,
                    jump: CulpritJump {
                        from: p.id,
                        takeoff_x: start_x.floor() as i32,
                        takeoff_offset: start_x.fract(),
                        dir,
                        // A climb is a sequence of full-height wall jumps, which is what the
                        // report should say it was rather than naming a single hold.
                        hold: JumpHold::Full,
                        speed: LaunchSpeed::Run,
                        to: traj.landed_on.filter(|q| *q != p.id),
                    },
                    climb: Some(dir),
                    swim: None,
                    vine: None,
                });
            }
        }
        // The two media. Both are swept from a platform's own ends only — water is entered off
        // an edge and a vine is grabbed from under it — so the cost is a handful of scripts per
        // platform rather than the jump sweep's whole grid.
        if options.swim {
            for (start_x, dir) in swim_starts(level, p) {
                for dive in [false, true] {
                    let traj = simulate_swim_with(
                        level,
                        landing,
                        start_x,
                        p.surface_row,
                        dir,
                        dive,
                        *options,
                    );
                    edges.push(MoveEdge {
                        traj,
                        jump: CulpritJump {
                            from: p.id,
                            takeoff_x: start_x.floor() as i32,
                            takeoff_offset: start_x.fract(),
                            dir,
                            hold: if dive { JumpHold::Full } else { JumpHold::None },
                            speed: LaunchSpeed::Run,
                            to: traj.landed_on.filter(|q| *q != p.id),
                        },
                        climb: None,
                        swim: Some((dir, dive)),
                        vine: None,
                    });
                }
            }
        }
        if options.vine {
            for start_x in vine_starts(level, p) {
                for exit in [TakeoffDir::Left, TakeoffDir::Right] {
                    let traj = simulate_vine_climb_with(
                        level,
                        landing,
                        start_x,
                        p.surface_row,
                        exit,
                        *options,
                    );
                    edges.push(MoveEdge {
                        traj,
                        jump: CulpritJump {
                            from: p.id,
                            takeoff_x: start_x.floor() as i32,
                            takeoff_offset: start_x.fract(),
                            dir: exit,
                            hold: JumpHold::Full,
                            speed: LaunchSpeed::Rest,
                            to: traj.landed_on.filter(|q| *q != p.id),
                        },
                        climb: None,
                        swim: None,
                        vine: Some(exit),
                    });
                }
                // The third way out of a vine: **over the block it hangs from.** A vine beside a
                // ledge is left sideways (the two moves above); a vine whose run ends under a
                // one-tile lid has no sideways exit at all, and the pull-up is the only way on.
                // Swept from the same take-off only when the geometry answers yes, so the graph is
                // not given a climb that ends nowhere on every ordinary vine.
                let col = Level::tile_x(start_x + PHYSICS.player_w * 0.5);
                let mantle = vine_run(level, col, p.surface_row, &PHYSICS)
                    .and_then(|(_, top)| vine_mantle_anchor(level, &PHYSICS, col, top));
                if mantle.is_some() {
                    let traj = simulate_vine_climb_with(
                        level,
                        landing,
                        start_x,
                        p.surface_row,
                        TakeoffDir::Still,
                        *options,
                    );
                    edges.push(MoveEdge {
                        traj,
                        jump: CulpritJump {
                            from: p.id,
                            takeoff_x: start_x.floor() as i32,
                            takeoff_offset: start_x.fract(),
                            // Straight up, with nothing pressed sideways: the climb is the arm and
                            // the pull-up at its top is the move.
                            dir: TakeoffDir::Still,
                            hold: JumpHold::Full,
                            speed: LaunchSpeed::Rest,
                            to: traj.landed_on.filter(|q| *q != p.id),
                        },
                        climb: None,
                        swim: None,
                        vine: Some(TakeoffDir::Still),
                    });
                }
            }
        }
        out.push(edges);
    }
    out
}

/// The canonical moves that leave one column of `platform`: the sub-tile take-off sweep in
/// every direction, and the walk-off at each end.
fn edges_at_column(
    level: &Level,
    platforms: &[Platform],
    p: &Platform,
    options: &ReachOptions,
    col: i32,
) -> Vec<MoveEdge> {
    let ph = &PHYSICS;
    let mut out = Vec::new();
    for &offset in &TAKEOFF_OFFSETS {
        let x = col as f32 + offset;
        // **A take-off is a position a body can be at.** The sweep chooses sub-tile offsets along
        // every column of a stand, and a body is a box: on the last column before a riser, the
        // offsets that put the box *inside* the step are not positions a walk can reach, and
        // simulating from them is worse than useless — the physics ejects the body out of the tile
        // and the oracle records a landing a player could never make. Found by walking
        // `l2-first-gap`'s three-tile staircase with `evo::route_play`: the graph offered a *still*
        // jump from inside the step as the cheapest way up, and a body that pressed it there just
        // bounced. See `takeoff_fits`.
        if !takeoff_fits(level, x, p.surface_y, ph) {
            continue;
        }
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
                    let traj =
                        simulate_jump_with(level, platforms, x, p.surface_y, &takeoff, *options);
                    out.push(MoveEdge {
                        traj,
                        jump: CulpritJump {
                            from: p.id,
                            takeoff_x: col,
                            takeoff_offset: offset,
                            dir,
                            hold: *hold,
                            speed: *speed,
                            to: traj.landed_on.filter(|q| *q != p.id),
                        },
                        climb: None,
                        swim: None,
                        vine: None,
                    });
                }
            }
            // Walking off the edge, only from the ends.
            let at_edge = col == p.x0 || col == p.x1;
            if at_edge && dir != TakeoffDir::Still {
                let takeoff =
                    Takeoff::with_runway(dir, JumpHold::None, LaunchSpeed::Run, runway, ph);
                let traj = simulate_jump_with(level, platforms, x, p.surface_y, &takeoff, *options);
                out.push(MoveEdge {
                    traj,
                    jump: CulpritJump {
                        from: p.id,
                        takeoff_x: col,
                        takeoff_offset: offset,
                        dir,
                        hold: JumpHold::None,
                        speed: LaunchSpeed::Run,
                        to: traj.landed_on.filter(|q| *q != p.id),
                    },
                    climb: None,
                    swim: None,
                    vine: None,
                });
            }
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
///
/// This is the question the editor asks on every edit and `train-cli` asks of every level it
/// is pointed at, and the sweep behind it is a whole level's canonical moves — so when the
/// level came from a file, the answer is looked for beside that file first and kept there for
/// the next process (`Level::cached_verdict`, and the oracle's own note on the cache in
/// [`crate::route_cache`]). Two things make that safe to do: the key is the level's content
/// hash and both behaviour versions, so an answer can only ever be reused for terrain and a
/// move set it was computed on, and a cache can only ever return what this function would
/// have returned — it is the same code that wrote it.
///
/// The cache holds the oracle's *own* question only: [`ReachOptions::default`]. A verdict for
/// weakened abilities is a different claim about the same terrain, and caching each of those
/// would rewrite one file per ability set for the benefit of nobody (the callers asking for
/// one are tests, and a test can afford the sweep).
pub fn report_with(level: &Level, options: ReachOptions) -> ReachReport {
    if let Some(cached) = level.cached_verdict(options) {
        return cached;
    }
    let report = compute_report(level, options);
    level.remember_verdict(options, &report);
    report
}

/// [`report_with`] with no cache in the way: the sweep, the BFS and the diagnosis.
fn compute_report(level: &Level, options: ReachOptions) -> ReachReport {
    // The verdict's own table: the terrain, and both ends of every moving platform's travel
    // ([`reach_platforms`]). The route graph keeps the terrain-only table on purpose — a ride
    // is a move with a clock in it and a route plan has no way to press it — but a *verdict* is
    // a statement that the level can be beaten, and a level whose only crossing is a shelf can.
    let platforms = reach_platforms(level);

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
            .find(|p| p.contains_col(col) && p.surface_y >= g.y)
            .map(|p| p.id)
    });

    let mut reached = vec![false; platforms.len()];
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); platforms.len()];
    let mut touched_goal_from: Vec<bool> = vec![false; platforms.len()];
    let mut all_edges: Vec<Vec<CulpritJump>> = vec![Vec::new(); platforms.len()];

    for (p, edges) in platforms
        .iter()
        .zip(edges_from_all(level, &platforms, &platforms, &options))
    {
        for edge in edges {
            all_edges[p.id].push(edge.jump);
            if edge.traj.touched_goal {
                touched_goal_from[p.id] = true;
            }
            if let Some(q) = edge.traj.landed_on {
                if q != p.id && !adjacency[p.id].contains(&q) {
                    adjacency[p.id].push(q);
                }
            }
        }
        // The ride: a body standing on a moving platform can wait for the cycle and be carried
        // to the other end of its travel, in either direction. It is an edge rather than a
        // simulated move because the phase is a fact about the episode, not the terrain — and
        // it is sound for the question this report answers: the cycle never stops, so "stand on
        // the deck and hold still" is always available.
        if let Some(partner) = p.ride {
            if !adjacency[p.id].contains(&partner) {
                adjacency[p.id].push(partner);
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

    // The next surface may be **above** the reachable one rather than beside it, and a level
    // whose route climbs is exactly the level a "gap to the right" reading misdescribes. So a
    // platform qualifies when it is not reached, not below the frontier, and either starts past
    // the frontier's right edge or rises above it; the pick is the nearest on both axes.
    let target = platforms
        .iter()
        .filter(|p| p.id != from.id && !reached[p.id])
        .filter(|p| {
            p.surface_row <= from.surface_row
                && (p.x0 > from.x1 || p.surface_row < from.surface_row)
        })
        .min_by_key(|p| {
            (
                (p.x0 - from.x1).max(0),
                (from.surface_row - p.surface_row).abs(),
            )
        });

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

/// The route to the flag as the move graph knows it: which stands are one crossing
/// apart, and how many crossings each stand is from the flag (§9.5).
///
/// [`ReachReport`] answers "can this level be beaten"; this answers **which way the
/// route goes**, and the difference is the whole reason it exists. A straight-line
/// reading — "the next stand is the one that lies closer to the flag" — is wrong on any
/// level that doubles back: on a zig-zag climb the stand the route visits next often
/// lies *farther* from the flag in a straight line than the stand behind it, and the
/// stands between two legs read closer to the flag than either end of the route.
/// Finding 50's tower arm caught that guess at work — a practice flag placed on the
/// ground the body had just climbed from, and one placed three crossings away — so the
/// ordering this replaces is measured wrong, not merely imprecise. Graph distance is
/// monotone along the route by construction: the crossings left to the flag only ever
/// fall as the run goes on.
///
/// Nodes are **landing zones** — the [`Level::landing_zone_bounds`] runs of standable
/// ground, which is the unit "a crossing" is already defined on
/// ([`Level::checkpoint_zone`]): walking is free inside a zone, and leaving one is a
/// jump, a hop over a hazard, or a drop. [`Platform`]s are coarser — a spike at standing
/// height splits one platform into two stands a body cannot walk between — so the graph
/// is built per zone, sweeping each zone's own columns with the very same canonical
/// moves [`report`] uses. The move set is never duplicated: every edge here is an
/// [`edges_from`] trajectory, so the route can only ever claim a move the game has.
/// One canonical crossing in the route graph: which stand it reaches, and **the take-off that
/// makes it**.
///
/// The graph used to keep only the landing stand, which was enough to answer "can the flag be
/// reached" and "which way is next". It is not enough to *walk the route*: a caller that wants to
/// drive a body along the graph needs the move itself — where the body stood, which way it went,
/// how long the jump was held and how fast the runway let it leave — and re-deriving that by
/// sweeping the canonical moves a second time is exactly the duplication this module refuses (the
/// move set has one implementation, [`edges_from_all`]). So the plan rides on the edge, and a route
/// becomes executable rather than merely countable: see `evo::route_play`.
///
/// The stored plan is the **least demanding** move found for that landing (a standing jump over a
/// sprinting one, a walk-off over a held jump), because a landing several moves can reach should be
/// replayed the easy way. Which move is kept never changes what the graph *answers*:
/// [`GoalRoutes::hops_to_flag`] and the waypoint sensor read [`RouteEdge::to`] and nothing else, so a
/// plan is an addition to the graph's memory rather than a change to its search.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteEdge {
    /// The stand this crossing lands on, or `None` for a move that only *touches* the flag in
    /// flight — a flag hanging over a pit is won without landing under it.
    pub to: Option<usize>,
    /// Whether the move touches the flag at all, which is what makes a hop of `1`.
    pub touches_flag: bool,
    /// World x of the body's left edge at the take-off.
    pub takeoff_x: f32,
    /// The surface row the take-off stands on.
    pub surface_row: i32,
    /// What to *do* at `takeoff_x`: press one canonical take-off, or run the climb script against
    /// the wall on one side.
    pub plan: RouteMove,
    /// `(column, surface_row)` the body came to rest on, when it landed at all.
    pub landed_at: Option<(i32, i32)>,
}

/// What a route edge asks the body to do at its take-off.
///
/// Two things, because the graph holds two things: a jump (or a walk-off, which is a take-off with
/// no jump in it) is *one press*, and a wall climb is a script — a rhythm of repeated wall jumps
/// that no single take-off reproduces. A plan that could not tell them apart would send a replay up
/// to a wall and press once.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum RouteMove {
    /// Press this take-off: direction, hold and speed.
    Jump(Takeoff),
    /// Climb the wall on this side, from the take-off position.
    Climb { dir: TakeoffDir },
    /// Swim this way from the take-off position; `dive` is a jump into the pool rather than a
    /// walk off the lip.
    Swim { dir: TakeoffDir, dive: bool },
    /// Grab the vine overhead and climb it, leaving toward `exit`.
    ///
    /// [`TakeoffDir::Still`] is the third exit: a vine that hangs from a **block** with air above it is
    /// climbed straight up and pulled over, and the body ends on the block rather than beside it.
    Vine { exit: TakeoffDir },
}

impl RouteEdge {
    /// How much the move asks of the body, smallest first: speed, then hold.
    ///
    /// This is a *replay* preference and not a solver's. A route is walked by a body that has to
    /// build whatever speed the move needs, so of two moves that reach the same stand the cheaper
    /// one is kept — and a climb is the dearest of all, because it is many presses rather than one
    /// and only worth taking when nothing simpler reaches the same ledge.
    fn demand(&self) -> (u8, u8) {
        match self.plan {
            RouteMove::Climb { .. } => (3, 3),
            // A vine is a climb: many presses, and only worth taking when nothing simpler
            // reaches the same ledge.
            RouteMove::Vine { .. } => (3, 2),
            // A swim is a walk that happens to be wet: it asks for no jump (a dive asks for one
            // and is still a single press), and its price is time rather than demand.
            RouteMove::Swim { dive, .. } => (0, u8::from(dive)),
            RouteMove::Jump(t) => {
                let speed = match t.speed {
                    LaunchSpeed::Rest => 0,
                    LaunchSpeed::Run => 1,
                    LaunchSpeed::Sprint => 2,
                };
                let hold = match t.hold {
                    JumpHold::None => 0,
                    JumpHold::Short => 1,
                    JumpHold::Full => 2,
                };
                (speed, hold)
            }
        }
    }

    /// How far **inside** the landing stand this move puts the body, in tiles, measured from the
    /// edge it arrives over: the margin a real body has, as opposed to a pixel it can hit.
    ///
    /// This exists because the walk found the alternative. Of the take-offs that reach a far ledge,
    /// the cheapest is the *first* one that clears — the one whose landing is a hair over the pit's
    /// edge, where a body that leaves a tenth of a tile early falls in. `l6-marathon`'s first pit is
    /// exactly that move, and a plan chosen for cost alone hands a player (and a replay) a window
    /// they cannot reliably hit. Preferring the landing further inside the stand is the same move
    /// made *with* margin, and it is free: those take-offs were swept too.
    fn landing_margin(&self, landing: (i32, i32, i32)) -> i32 {
        let Some((col, _)) = self.landed_at else {
            return 0;
        };
        let dir = match self.plan {
            RouteMove::Jump(Takeoff { dir, .. }) => dir,
            RouteMove::Swim { dir, .. } => dir,
            RouteMove::Vine { exit } => exit,
            RouteMove::Climb { .. } => TakeoffDir::Still,
        };
        match dir {
            TakeoffDir::Left => landing.2 - col,
            // A climb arrives at the wall's top and a still jump comes down where it left: neither
            // crosses anything, so neither has a margin to speak of. A vine's landing *is* a
            // crossing (off the vine, onto the ledge), so it keeps its margin.
            TakeoffDir::Still => 0,
            TakeoffDir::Right => col - landing.1,
        }
    }
}

/// Whether `candidate` is a better move to *keep* for a landing than the `held` one.
///
/// Cost first (see [`RouteEdge::demand`]), then margin (see [`RouteEdge::landing_margin`]). A tie keeps
/// the move the sweep found first, which is what "the sweep's order is fixed" means: no position is
/// privileged beyond the two rules above. There used to be a third term breaking ties by take-off
/// position, and it was wrong in both directions — preferring the *later* take-off walked the replay to
/// the very lip of a stand, where it could not press and stalled with the hop uncrossed on `l2`..`l5`;
/// preferring the *earlier* one moved `l9-wait-and-climb`'s first crossing to a take-off the four-tick
/// cadence cannot stop on (aim 6.00, pressed at 6.15), and the walk died at the ledge. Both are the
/// same discovery: which swept position an edge stores is a *move-set* decision, not a cosmetic one.
/// See `MOVE_SET_VERSION` 4.
fn better_plan(candidate: &RouteEdge, held: &RouteEdge, landing: (i32, i32, i32)) -> bool {
    candidate
        .demand()
        .cmp(&held.demand())
        .then(
            held.landing_margin(landing)
                .cmp(&candidate.landing_margin(landing)),
        )
        .is_lt()
}

/// One standable surface in the route graph: ground, or one end of a moving platform's travel.
///
/// `row` and `x0..x1` are what [`GoalRoutes::zone_at`] is keyed on and what a landing reports; the
/// two extra fields are what a deck needs and ground does not. `surface_y` is the top edge — the
/// cell's own top for terrain, **mid-cell** for a deck, because a platform's box is half a tile tall
/// and sits on the bottom of its row ([`crate::level::PlatformSpec`]). `ride` names the deck at the
/// other end of the travel, which is the one edge of this graph that is not a move: nobody presses
/// it, the cycle makes it true.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stand {
    pub row: i32,
    pub x0: i32,
    pub x1: i32,
    pub surface_y: f32,
    pub ride: Option<usize>,
}

impl Stand {
    /// Ground: a cell row, its top edge on the cell's own boundary, no ride.
    fn ground(row: i32, x0: i32, x1: i32) -> Stand {
        Stand {
            row,
            x0,
            x1,
            surface_y: row as f32,
            ride: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoalRoutes {
    /// Every stand in the level, in scan order: the terrain's first, then a moving platform's
    /// decks. See [`Stand`].
    stands: Vec<Stand>,
    /// Index-aligned with `stands`: the canonical crossings that leave the stand, each with the
    /// take-off that makes it. At most one entry per landing stand.
    crossings: Vec<Vec<RouteEdge>>,
    /// Index-aligned with `stands`: crossings from the stand to the flag — `None` where
    /// the flag cannot be reached from there at all.
    hops: Vec<Option<u32>>,
}

impl GoalRoutes {
    /// Total number of standable landing zones in the level.
    pub fn len(&self) -> usize {
        self.stands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stands.is_empty()
    }

    /// The stand containing the ground at `(col, surface_row)`.
    ///
    /// Terrain is searched first, so a body whose tile is over both a floor and a deck that has
    /// slid across it reads the floor: the deck is a place to be *carried*, and a stand that moves
    /// is only ever the honest answer when there is nothing fixed under the feet.
    pub fn zone_at(&self, col: i32, surface_row: i32) -> Option<usize> {
        self.stands
            .iter()
            .position(|s| s.row == surface_row && col >= s.x0 && col <= s.x1)
    }

    /// The stand's `(surface_row, x0, x1)`.
    pub fn zone_bounds(&self, stand: usize) -> (i32, i32, i32) {
        let s = self.stands[stand];
        (s.row, s.x0, s.x1)
    }

    /// The world `y` of the stand's top edge — mid-cell for a moving platform's deck, the cell's own
    /// top for ground. A jump off a deck starts half a tile above the row its stand is named for.
    pub fn zone_surface(&self, stand: usize) -> f32 {
        self.stands[stand].surface_y
    }

    /// The stand at the other end of this stand's ride, when the stand is a moving platform's deck.
    ///
    /// A ride is not a crossing and has no [`RouteEdge`]: there is nothing to press. It says "wait
    /// for the cycle", which is the one move in this game whose action is not in the action set and
    /// the reason [`Self::next_waypoint`] aims at the body itself for it.
    pub fn ride_from(&self, stand: usize) -> Option<usize> {
        self.stands[stand].ride
    }

    /// The canonical crossings that leave `from`, with the move that makes each one. A jump that
    /// lands back where it started is not a crossing and is not listed.
    pub fn crossings_from(&self, from: usize) -> &[RouteEdge] {
        &self.crossings[from]
    }

    /// The stands one canonical crossing from `from` lands on.
    pub fn crossing_stands_from(&self, from: usize) -> impl Iterator<Item = usize> + '_ {
        self.crossings[from].iter().filter_map(|e| e.to)
    }

    /// The move that reaches the flag from `stand`, when a crossing from there touches it.
    ///
    /// This is the **last** move of a route: a stand with a flag-touching crossing has
    /// `hops_to_flag == 1` even when nothing under the flag is standable, so a caller walking the
    /// route needs this move rather than a waypoint (which points at a landing, and there is none).
    pub fn flag_edge(&self, stand: usize) -> Option<&RouteEdge> {
        self.crossings[stand].iter().find(|e| e.touches_flag)
    }

    /// Crossings from `stand` to the flag. `Some(0)` is the flag's own stand; `None`
    /// means no route to the flag from here at all.
    pub fn hops_to_flag(&self, stand: usize) -> Option<u32> {
        self.hops[stand]
    }

    /// The **move** the route makes next from `stand`, chosen exactly as
    /// [`Self::next_waypoint`] chooses its landing: the next stand reachable in one canonical
    /// crossing that strictly decreases `hops_to_flag`, tie-broken on proximity to `from_x` and then
    /// to `goal`.
    ///
    /// `None` when the stand is the flag's own ground (`hops == 0`) or has no route at all.
    pub fn next_hop(&self, stand: usize, from_x: f32, goal: Option<Rect>) -> Option<&RouteEdge> {
        self.aim(stand, from_x, goal).and_then(|(_, edge)| edge)
    }

    /// The world-space `(x, y)` waypoint the route aims at next from `stand`, when one
    /// exists.
    ///
    /// If `stand` is already on the flag's own ground (`hops == 0`), or has no route to
    /// the flag (`hops == None`), returns `None` so the caller aims directly at the goal.
    /// Otherwise picks the next stand `j` reachable in one canonical crossing from `stand`
    /// that strictly decreases `hops_to_flag` (with tie-breaking on proximity to `from_x`
    /// and then to `goal`), and returns the world centre of the landing tile in `j` closest
    /// to `from_x`.
    pub fn next_waypoint(
        &self,
        stand: usize,
        from_x: f32,
        goal: Option<Rect>,
    ) -> Option<(f32, f32)> {
        self.aim(stand, from_x, goal).map(|(point, _)| point)
    }

    /// The one implementation of "which crossing does the route take next": the aim point the
    /// waypoint sensor steers at, and the move a replay issues when it gets there.
    ///
    /// **The two callers must not drift.** The sensor's steering point and a replayed route are the
    /// same decision, so they are the same code; a second copy of this selection would be a
    /// waypoint sensor that steers at a landing the replay does not jump to. The tie-break keeps the
    /// *first* candidate on equal keys, which is what makes the answer independent of insertion
    /// order in the map iteration below it.
    fn aim(
        &self,
        stand: usize,
        from_x: f32,
        goal: Option<Rect>,
    ) -> Option<((f32, f32), Option<&RouteEdge>)> {
        let from_hops = self.hops.get(stand).copied().flatten()?;
        if from_hops == 0 {
            return None;
        }
        let (from_row, fx0, fx1) = self.zone_bounds(stand);
        let from_col = (from_x.floor() as i32).clamp(fx0, fx1);
        // `(hops, distance from the body, distance to the goal, aim point, edge)`: the sort key is
        // the first three and the rest is what the caller asked for, kept together so the loop
        // updates one binding rather than three parallel ones.
        type Aim<'a> = (u32, f32, f32, (f32, f32), Option<&'a RouteEdge>);
        // A free function rather than a closure: the aim point is a reference to an edge of
        // `self.crossings`, and a closure that stores one has to name the borrow it is storing.
        fn consider<'a>(
            best: &mut Option<Aim<'a>>,
            from_hops: u32,
            goal: Option<Rect>,
            hops: u32,
            d_from: f32,
            point: (f32, f32),
            edge: Option<&'a RouteEdge>,
        ) {
            if hops >= from_hops {
                return;
            }
            let d_goal = goal.map_or(0.0, |g| {
                let (gx, gy) = g.center();
                (point.0 - gx).hypot(point.1 - gy)
            });
            let key = (hops, d_from, d_goal);
            if best.is_none_or(|(bh, bf, bg, _, _)| key < (bh, bf, bg)) {
                *best = Some((hops, d_from, d_goal, point, edge));
            }
        }
        let mut best: Option<Aim> = None;
        for edge in &self.crossings[stand] {
            let Some(j) = edge.to else {
                continue;
            };
            let Some(hops) = self.hops[j] else {
                continue;
            };
            let (row, x0, x1) = self.zone_bounds(j);
            let target_col = from_col.clamp(x0, x1);
            let point = (target_col as f32 + 0.5, self.zone_surface(j) - 0.5);
            let d_from = ((target_col - from_col) as f32).hypot((row - from_row) as f32);
            consider(&mut best, from_hops, goal, hops, d_from, point, Some(edge));
        }
        // The ride: a stand whose next step is being carried waits where it stands, so its aim
        // point is the body itself — a zero waypoint, which is the only honest way to say "hold
        // still" in a vector whose two waypoint slots are a direction. Distance zero also means a
        // ride wins a tie against a crossing at the same hop count, which is right: a body that is
        // already on the deck has nothing to press.
        if let Some(partner) = self.ride_from(stand) {
            if let Some(hops) = self.hops[partner] {
                consider(
                    &mut best,
                    from_hops,
                    goal,
                    hops,
                    0.0,
                    (from_x, self.zone_surface(stand) - 0.5),
                    None,
                );
            }
        }
        best.map(|(_, _, _, pt, edge)| (pt, edge))
    }
}

/// Build the route graph for a level (§9.5).
///
/// The flag counts as reached from a stand whose crossing can **touch** it, not only
/// from the one it stands on: a flag floating over a pit is won in flight, and a route
/// graph that only knew how to land under it would call such a level unrouteable.
pub fn goal_routes(level: &Level) -> GoalRoutes {
    let mut stands: Vec<Stand> = Vec::new();
    for row in 0..level.height {
        let mut x = 0;
        while x < level.width {
            if !level.is_standable(x, row) {
                x += 1;
                continue;
            }
            let (x0, x1) = level.landing_zone_bounds(x, row);
            stands.push(Stand::ground(row, x0, x1));
            x = x1 + 1;
        }
    }

    // **A moving platform is two stands and a ride.** The terrain's stands are built first, so a
    // body whose tile is over both floor and deck reads the floor; then each deck at the near end of
    // a travel and each deck at the far end are added, with `ride` pointing at each other. That pair
    // is what lets the compass say "jump onto the deck, then wait", and — the reason it is here at
    // all — what lets `curriculum::next_stand` answer a stall on a shuttle's ledge instead of
    // reporting that the flag cannot be reached. Before this, a run that stalled at a deck was left
    // with no practice level and no plan: the population that produced this graph's first finding
    // logs that sentence fifty generations in a row.
    //
    // The landing table is [`reach_platforms`] for the same reason the verdict's is: a jump onto a
    // deck has to be a crossing this graph can see, or the first stand after the ledge is
    // unreachable and nothing downstream matters.
    let landing = reach_platforms(level);
    let mut landing_stand: Vec<Option<usize>> = vec![None; landing.len()];
    for p in landing.iter().filter(|p| p.ride.is_none()) {
        landing_stand[p.id] = stands.iter().position(|s| {
            s.ride.is_none()
                && s.row == p.surface_row
                && p.x0 >= s.x0
                && p.x1 <= s.x1
                && s.x0 >= p.x0 - 1
                && s.x1 <= p.x1 + 1
        });
    }
    let decks: Vec<usize> = landing
        .iter()
        .filter(|p| p.ride.is_some())
        .map(|p| p.id)
        .collect();
    for &id in &decks {
        let p = landing[id];
        landing_stand[id] = Some(stands.len());
        stands.push(Stand {
            row: p.surface_row,
            x0: p.x0,
            x1: p.x1,
            surface_y: p.surface_y,
            ride: None,
        });
    }
    for &id in &decks {
        let mine = landing_stand[id];
        let partner = landing[id].ride.and_then(|r| landing_stand[r]);
        if let (Some(mine), Some(partner)) = (mine, partner) {
            stands[mine].ride = Some(partner);
        }
    }

    let stand_of = |c: i32, lrow: i32| -> Option<usize> {
        stands
            .iter()
            .position(|s| s.row == lrow && c >= s.x0 && c <= s.x1)
    };

    // Every edge is one [`edges_from_all`] trajectory — the report's own move set — swept from
    // each stand's own columns (the synthetic take-off platform *is* the stand; its id only feeds
    // `CulpritJump`, which this graph has no use for), while `landing` is what a trajectory can
    // land on.
    let options = ReachOptions::default();
    let mut crossings: Vec<Vec<RouteEdge>> = vec![Vec::new(); stands.len()];
    let mut touches_flag = vec![false; stands.len()];
    let takeoffs: Vec<Platform> = stands
        .iter()
        .enumerate()
        .map(|(i, s)| Platform {
            id: i,
            x0: s.x0,
            x1: s.x1,
            surface_row: s.row,
            surface_y: s.surface_y,
            ride: None,
        })
        .collect();
    for (i, edges) in edges_from_all(level, &landing, &takeoffs, &options)
        .into_iter()
        .enumerate()
    {
        for edge_move in edges {
            let (traj, jump) = (edge_move.traj, edge_move.jump);
            if traj.touched_goal {
                touches_flag[i] = true;
            }
            // A jump that landed back where it started is not a crossing, and one that only fell
            // is not either: an edge exists because it *arrived* somewhere — on a stand, at the
            // flag, or both.
            let to = traj
                .landed_at
                .and_then(|(c, lrow)| stand_of(c, lrow))
                .filter(|j| *j != i);
            if to.is_none() && !traj.touched_goal {
                continue;
            }
            let edge = RouteEdge {
                to,
                touches_flag: traj.touched_goal,
                takeoff_x: jump.takeoff_x as f32 + jump.takeoff_offset,
                surface_row: stands[i].row,
                plan: match (edge_move.climb, edge_move.swim, edge_move.vine) {
                    (Some(dir), _, _) => RouteMove::Climb { dir },
                    (_, Some((dir, dive)), _) => RouteMove::Swim { dir, dive },
                    (_, _, Some(exit)) => RouteMove::Vine { exit },
                    _ => RouteMove::Jump(Takeoff::with_runway(
                        jump.dir,
                        jump.hold,
                        jump.speed,
                        runway_behind(&takeoffs[i], jump.takeoff_x, jump.dir),
                        &PHYSICS,
                    )),
                },
                landed_at: traj.landed_at,
            };
            // One edge per landing stand (and one for a flag touch that lands nowhere), holding
            // the least demanding move that reaches it. Deterministic: the first move found wins a
            // tie, and the sweep's order is fixed.
            match to {
                Some(j) => match crossings[i].iter_mut().find(|e| e.to == Some(j)) {
                    Some(slot) => {
                        if better_plan(&edge, slot, (stands[j].row, stands[j].x0, stands[j].x1)) {
                            *slot = edge;
                        }
                    }
                    None => crossings[i].push(edge),
                },
                None => {
                    if !crossings[i].iter().any(|e| e.to.is_none()) {
                        crossings[i].push(edge);
                    }
                }
            }
        }
    }

    // Crossings to the flag, walked backwards — a breadth-first search from a virtual
    // flag node with an edge from the flag's own stand (0 crossings) and from every
    // stand whose crossing touches the flag (1). Deterministic: neighbours are visited
    // in insertion order.
    let goal_stand = level.goals.first().and_then(|g| {
        let col = (g.x + g.w * 0.5).floor() as i32;
        stands
            .iter()
            .enumerate()
            // The flag's column, **and a surface a body standing there can reach it from.** The
            // column test alone was wrong for a flag near the top of a level: the ground seven
            // tiles below it shares the column, so it was taken as the flag's own stand, and the
            // route graph then answered "you are already home" to a body standing under a vine it
            // had to climb. A flag whose ground is out of standing reach is not that stand's goal —
            // the move that wins is the *crossing* that touches it in flight (`touches_flag`), and
            // it keeps its own hop count.
            .filter(|(_, s)| {
                col >= s.x0 && col <= s.x1 && g.y + g.h >= s.surface_y - PHYSICS.player_h
            })
            .min_by_key(|(_, s)| s.row)
            .map(|(i, _)| i)
    });
    let mut reached_from: Vec<Vec<usize>> = vec![Vec::new(); stands.len()];
    for (i, outs) in crossings.iter().enumerate() {
        for edge in outs {
            if let Some(j) = edge.to {
                reached_from[j].push(i);
            }
        }
    }
    // The rides are edges in this search and in nothing else: a stand you are carried to has one
    // fewer crossing left, exactly as if a move had put you there, so a shuttle's far deck is the
    // next rung of the route rather than the end of it.
    for (i, s) in stands.iter().enumerate() {
        if let Some(j) = s.ride {
            reached_from[j].push(i);
        }
    }
    let mut hops: Vec<Option<u32>> = vec![None; stands.len()];
    let mut queue = std::collections::VecDeque::new();
    for (i, touches) in touches_flag.iter().enumerate() {
        if *touches {
            hops[i] = Some(1);
            queue.push_back(i);
        }
    }
    if let Some(g) = goal_stand {
        hops[g] = Some(0);
        queue.push_front(g);
    }
    while let Some(j) = queue.pop_front() {
        let h = hops[j].expect("a queued stand has a hop count");
        for &i in &reached_from[j] {
            if hops[i].is_none() {
                hops[i] = Some(h + 1);
                queue.push_back(i);
            }
        }
    }

    GoalRoutes {
        stands,
        crossings,
        hops,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A pad on the lip of a seven-tile pit: the shape the owner reported building by hand
    // (`REVIEW.md` §1.12). Seven columns is past a sprint jump's 6.00 tiles — and the body is 0.6
    // wide, which is the margin that makes six columns crossable and seven not — so from the ground
    // this pit cannot be crossed at all. The pad is what crosses it: a body that comes down on the
    // pad is thrown 4.9 tiles up and carried 56 ticks at the speed it arrived with, 8.4 tiles of
    // that at a sprint.
    const PAD_GAP: &[&str] = &[
        "........................",
        "........................",
        "........................",
        "........................",
        "........................",
        "........................",
        "...........v............",
        "############.......#####",
    ];

    #[test]
    fn a_pad_at_the_lip_is_what_crosses_a_gap_no_jump_can() {
        let lv = level("pad-gap", PAD_GAP, (1, 6), &[("goal", 21, 6)]);

        let with_pad = report(&lv);
        assert!(with_pad.goal_reached, "{:?}", with_pad.diagnosis);

        // With pads withheld the same terrain is uncrossable, so the pad is not a convenience
        // here — it is the move. This is the assertion that would have caught §1.12 before the
        // owner did: the oracle used to answer this level as *unbeatable* with every ability on.
        let without_pad = report_with(
            &lv,
            ReachOptions {
                spring: false,
                ..ReachOptions::default()
            },
        );
        assert!(
            !without_pad.goal_reached,
            "a seven-tile pit is past even a sprint jump, so without pads this must be \
             unreachable; it is reachable another way, and the fixture is not testing the pad"
        );
        let why = without_pad.diagnosis.clone().expect("a diagnosis");
        assert!(why.contains("gap"), "unexpected diagnosis: {why}");
    }

    // A pad on the floor's last column, with nothing beyond it: the shape that separates "a pad
    // throws a descending body" from "a pad throws anything that touches it".
    const PAD_LEDGE: &[&str] = &[
        "..........",
        "..........",
        "..........",
        ".....v....",
        "######....",
    ];

    /// A body that *walks* off a pad's ledge is not thrown, and the oracle has to agree — the pad's
    /// rule is about arriving on it, not about touching it. The half that makes this worth a test is
    /// that a walk-off take-off starts with the body grounded inside the pad's own cell: an oracle
    /// that asked "is the body overlapping a pad" instead of running the sim's rule would launch a
    /// body here, and would then credit a ledge like this with a move the game does not have.
    #[test]
    fn walking_off_a_pad_at_a_ledge_is_never_a_launch() {
        // The flag sits behind the spawn, out of a rightward run's way: this fixture is about a body
        // that leaves a ledge, and a goal in front of it would be a win before the walk-off happens.
        let lv = level("pad-ledge", PAD_LEDGE, (2, 3), &[("goal", 0, 3)]);

        // The sim's answer, driven by the sim: run right off the ledge, and never be thrown.
        let mut sim = crate::Sim::new(&lv, 3);
        let mut launch = false;
        for _ in 0..200 {
            sim.step(crate::AgentAction::Run);
            launch |= sim.player().vy == -PHYSICS.spring_v0;
            if sim.outcome().is_finished() {
                break;
            }
        }
        assert!(!launch, "a body walking over a pad is not landing on it");
        assert_eq!(
            sim.outcome(),
            crate::Outcome::Died,
            "the ledge ends in a pit"
        );

        // And the oracle's own canonical walk-off from that column is the same move: a fall.
        let platforms = extract_platforms(&lv);
        let walk = simulate_jump(
            &lv,
            &platforms,
            5,
            4,
            TakeoffDir::Right,
            JumpHold::None,
            LaunchSpeed::Run,
        );
        assert!(walk.fell_out, "walking off this ledge is a fall: {walk:?}");
        assert!(walk.landed_on.is_none(), "and it lands nowhere: {walk:?}");
    }

    /// The oracle's promise is that it can only ever disagree with the sim about whether a *move* is
    /// expressible in the action space — never about how far a move goes. So the crossing it credits
    /// to a pad has to be a crossing the real episode loop makes: this is that check, on the same
    /// terrain, driven by `Sim::step`, sprinting right and jumping once near the lip.
    ///
    /// It sweeps the take-off point rather than pinning one, because the *player's* question is not
    /// "jump at exactly 7.5" — it is "is there a moment in that last stretch that works", which is
    /// what the oracle's own 20-offset sweep answers and what a fixed threshold would quietly
    /// narrow.
    #[test]
    fn the_sim_crosses_the_gap_the_oracle_credits_to_the_pad() {
        let lv = level("pad-gap", PAD_GAP, (1, 6), &[("goal", 21, 6)]);
        let mut crossed = None;
        let mut launched_anywhere = false;
        for jump_at in [6.5f32, 7.5, 8.5, 9.5, 10.5] {
            let mut sim = crate::Sim::new(&lv, 4);
            let mut jumped = false;
            let mut launch = None;
            for _ in 0..1200 {
                let action = if !jumped && sim.player().x >= jump_at {
                    jumped = true;
                    crate::AgentAction::SprintJump
                } else {
                    crate::AgentAction::Sprint
                };
                sim.step(action);
                // `spring_v0` is 0.35 against a jump's 0.25, so a body leaving the ground this fast
                // has been thrown by a pad rather than by its own legs.
                if sim.player().vy == -PHYSICS.spring_v0 && launch.is_none() {
                    launch = Some(sim.player().x);
                }
                if sim.outcome().is_finished() {
                    break;
                }
            }
            if launch.is_some() {
                launched_anywhere = true;
            }
            // A jump taken too early drops the body into the pit before it ever reaches the pad,
            // so what this test needs is *one* take-off that does both — and a crossing that was
            // not thrown is the thing the oracle's own answer rules out.
            if sim.outcome() == crate::Outcome::Won {
                assert!(
                    launch.is_some(),
                    "a sprint from {jump_at} crossed the pit without a launch, so the oracle is \
                     crediting a pad for a jump's work"
                );
                crossed = Some((jump_at, launch.unwrap()));
            }
        }
        assert!(
            launched_anywhere,
            "no sprint in this sweep ever landed on the pad, so the fixture stopped testing one"
        );
        let (jump_at, at) = crossed.expect(
            "the oracle credits this level to its pad, so at least one of these sprints must cross \
             it; none did",
        );
        assert!(
            (11.0..12.0).contains(&at),
            "the run that crossed jumped at {jump_at} and was thrown from x = {at}, which is not the \
             pad's own column"
        );
    }

    #[test]
    fn the_route_counts_crossings_to_the_flag_and_not_straight_line_distance() {
        // The zig-zag: the route doubles back away from the flag up the first leg, so
        // straight-line distance does not fall as the run goes on — graph distance has
        // to, or "the next stand" has no meaning on a climb like this.
        let level = level(
            "zig",
            &[
                "..........",
                "..........",
                ".......##.",
                "..........",
                ".....##...",
                "..........",
                ".##.......",
                "..........",
                "....##....",
                "..........",
            ],
            (5, 7),
            &[("goal", 8, 1)],
        );
        let routes = goal_routes(&level);
        let bottom = routes.zone_at(5, 8).expect("the spawn's stand");
        let leg_top = routes.zone_at(2, 6).expect("the top of the first leg");
        let turn = routes
            .zone_at(5, 4)
            .expect("the first stand back toward the flag");
        let top = routes.zone_at(8, 2).expect("the flag's own stand");
        assert_eq!(routes.hops_to_flag(top), Some(0), "the flag's own stand");
        for (from, to) in [(bottom, leg_top), (leg_top, turn), (turn, top)] {
            assert!(
                routes.crossing_stands_from(from).any(|j| j == to),
                "a stand is one canonical crossing from the stand before it on the route"
            );
        }
        // The route's own numbering falls by one per stand — the only ordering that is
        // monotone along a climb that turns its back on the flag.
        let hops = |z: usize| routes.hops_to_flag(z).expect("the flag is reachable");
        assert!(
            hops(bottom) > hops(leg_top) && hops(leg_top) > hops(turn) && hops(turn) > hops(top),
            "crossings to the flag must fall along the route: {:?}",
            [hops(bottom), hops(leg_top), hops(turn), hops(top)]
        );
    }

    /// A parallel sweep answers exactly what a sequential one does.
    ///
    /// This is the crate's one use of threads, and it is the last place a determinism guarantee
    /// should be resting on a scheduler. A sweep's output is a *sequence* whatever else it is —
    /// the last jump out of a platform is the jump a stuck run is blamed on, and the route
    /// graph's crossing lists are built in edge order — so the chunks are assembled by index and
    /// this asserts the two answers agree edge for edge, order included.
    ///
    /// Both shapes of job are compared, because they are the two the crate actually asks for: a
    /// level's own platforms swept onto themselves (what [`report_with`] wants) and its stands
    /// swept as their own take-off platforms (what [`goal_routes`] wants). The fixture is wide
    /// enough for the threaded path to be the one under test, and that is asserted rather than
    /// assumed: a test that had quietly become two sequential sweeps would prove nothing.
    #[test]
    fn a_parallel_sweep_answers_exactly_what_a_sequential_one_does() {
        let level = level(
            "wide",
            &[
                "........................",
                "........................",
                ".........####...........",
                "........................",
                "....^^..................",
                "##########..############",
                "########################",
            ],
            (1, 5),
            &[("goal", 22, 5)],
        );
        let options = ReachOptions::default();
        let landing = extract_platforms(&level);

        // The stands `goal_routes` derives for itself, published through its own accessors.
        let routes = goal_routes(&level);
        let stands: Vec<Platform> = (0..routes.len())
            .map(|i| {
                let (surface_row, x0, x1) = routes.zone_bounds(i);
                Platform {
                    id: i,
                    x0,
                    x1,
                    surface_row,
                    surface_y: surface_row as f32,
                    ride: None,
                }
            })
            .collect();

        for takeoffs in [landing.clone(), stands] {
            let columns: usize = takeoffs
                .iter()
                .map(|p| (p.x1 - p.x0 + 1).max(0) as usize)
                .sum();
            assert!(
                columns >= MIN_PARALLEL_COLUMNS,
                "the fixture has to be wide enough to take the threaded path: {columns} columns"
            );
            let sequential = edges_from_all_with(&level, &landing, &takeoffs, &options, 1);
            let threaded = edges_from_all_with(&level, &landing, &takeoffs, &options, 2);
            assert_eq!(
                sequential, threaded,
                "two threads must sweep exactly what one sweeps, in the same order"
            );
            assert_eq!(
                edges_from_all(&level, &landing, &takeoffs, &options),
                sequential,
                "and the sweep a caller gets by default is that same answer"
            );
        }
    }

    #[test]
    fn a_hazard_splits_one_platform_into_two_stands_the_route_has_to_hop() {
        // A floor spike at standing height cuts the ground in two: the body cannot
        // walk through it, so the stands either side are one crossing apart even
        // though `extract_platforms` sees a single platform under both.
        let level = level(
            "spiked",
            &[
                "........", "........", "........", "........", "........", "...^....", "########",
                "########",
            ],
            (1, 5),
            &[("goal", 7, 5)],
        );
        let routes = goal_routes(&level);
        let near = routes.zone_at(1, 6).expect("the stand before the spike");
        let far = routes.zone_at(6, 6).expect("the stand past the spike");
        assert_ne!(near, far, "the spike splits the ground into two stands");
        assert_eq!(routes.zone_bounds(near), (6, 0, 2));
        assert_eq!(routes.zone_bounds(far), (6, 4, 7));
        assert_eq!(
            routes.hops_to_flag(far),
            Some(0),
            "the flag stands on the far side of the spike"
        );
        assert_eq!(routes.hops_to_flag(near), Some(1), "one hop away");
        assert!(
            routes.crossing_stands_from(near).any(|j| j == far),
            "a hop over the spike is a crossing — the two stands are not walking distance"
        );
    }

    #[test]
    fn a_move_cannot_start_inside_a_hazard_and_a_sealed_route_stays_sealed() {
        // The phantom edge: the take-off ledge's right column is spike-topped, so
        // sweeping take-off x across it authors a body standing *inside* the spike.
        // The hazard check runs after movement, so such a body leaves the spike's tile
        // between tick zero and tick one and was never checked at all — inventing a
        // jump out of a spot no body can occupy, and with it a route up a tower the
        // game seals at that spike (finding 52).
        //
        // The flag stands high above the flight's reach — a jump from the ledge peaks
        // 2.5 tiles up — so the only way to it is over the spike, and there is none:
        // the cut is real, not an artefact of a short fixture.
        let lv = level(
            "sealed",
            &[
                "..........",
                "..........",
                "..........",
                "........##",
                "..........",
                "......##..",
                "..^.......",
                ".##.......",
                "..........",
            ],
            (1, 6),
            &[("goal", 8, 2)],
        );
        let platforms = extract_platforms(&lv);
        let phantom = simulate_jump_at(
            &lv,
            &platforms,
            2.90,
            7.0,
            &Takeoff::with_runway(
                TakeoffDir::Right,
                JumpHold::Full,
                LaunchSpeed::Run,
                1.0,
                &PHYSICS,
            ),
        );
        assert!(
            phantom.hit_spike && phantom.landed_on.is_none() && phantom.ticks == 0,
            "a take-off authored inside the spike is a death at the take-off, not a jump: {phantom:?}"
        );

        let routes = goal_routes(&lv);
        let takeoff = routes.zone_at(1, 7).expect("the stand below the spike");
        let target = routes.zone_at(6, 5).expect("the stand across the spike");
        assert_eq!(
            routes.hops_to_flag(target),
            Some(1),
            "one real jump from the far stand reaches the flag — the fixture is not sealed everywhere"
        );
        let outs: Vec<usize> = routes.crossing_stands_from(takeoff).collect();
        let reached: Vec<(i32, i32, i32)> = outs.iter().map(|&j| routes.zone_bounds(j)).collect();
        assert_eq!(
            routes.hops_to_flag(takeoff),
            None,
            "the route is sealed at the spike; the ledge only crosses to {reached:?}"
        );
        assert!(
            !report(&lv).goal_reached,
            "and the oracle says so instead of calling the level beatable"
        );
    }

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

    /// **Every edge of the route graph carries a move that really makes it.**
    ///
    /// A plan on an edge is worth having only if it is the oracle's own move rather than a guess:
    /// a replay drives a body to `takeoff_x`, presses the plan and expects to arrive where the
    /// edge says it does. So the invariant is checked against the move set itself — each edge's
    /// plan is simulated from its own take-off and required to produce the landing the edge
    /// claims, flag touches included. A graph that could not survive this would have a route that
    /// is not walkable, which is the one thing a route is for.
    #[test]
    fn every_route_edge_replays_to_the_stand_it_claims() {
        // A fixture: name, tile rows, spawn, flag entries.
        type Case<'a> = (
            &'a str,
            &'a [&'a str],
            (i32, i32),
            &'a [(&'a str, i32, i32)],
        );
        let cases: [Case; 4] = [
            ("gap", GAP, (1, 5), &[("goal", 19, 5)]),
            ("gap7", GAP7, (1, 5), &[("goal", 19, 5)]),
            ("ladder", LADDER, (1, 5), &[("goal", 7, 3)]),
            ("tower", TOWER, (1, 6), &[("goal", 7, 2)]),
        ];
        for (name, rows, spawn, ents) in cases {
            let lv = level(name, rows, spawn, ents);
            let routes = goal_routes(&lv);
            let platforms = extract_platforms(&lv);
            let mut checked = 0usize;
            for stand in 0..routes.len() {
                for edge in routes.crossings_from(stand) {
                    let traj = match edge.plan {
                        RouteMove::Jump(takeoff) => simulate_jump_at(
                            &lv,
                            &platforms,
                            edge.takeoff_x,
                            edge.surface_row as f32,
                            &takeoff,
                        ),
                        RouteMove::Climb { dir } => simulate_wall_climb(
                            &lv,
                            &platforms,
                            edge.takeoff_x,
                            edge.surface_row,
                            dir,
                        ),
                        RouteMove::Swim { dir, dive } => simulate_swim(
                            &lv,
                            &platforms,
                            edge.takeoff_x,
                            edge.surface_row,
                            dir,
                            dive,
                        ),
                        RouteMove::Vine { exit } => simulate_vine_climb(
                            &lv,
                            &platforms,
                            edge.takeoff_x,
                            edge.surface_row,
                            exit,
                        ),
                    };
                    let landed = traj.landed_at.and_then(|(c, row)| routes.zone_at(c, row));
                    assert_eq!(
                        landed, edge.to,
                        "{name}: the plan out of stand {stand} ({:?}) lands on {landed:?}, not the \
                         stand the graph says it reaches ({:?})",
                        edge.plan, edge.to
                    );
                    assert_eq!(
                        traj.touched_goal, edge.touches_flag,
                        "{name}: the flag touch on an edge out of stand {stand} does not replay"
                    );
                    checked += 1;
                }
            }
            assert!(
                checked > 0,
                "{name} has no route edges to check, so this test proves nothing"
            );
        }
    }

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
        assert_eq!(
            report.start,
            Some(1),
            "spawn should sit on the ground platform"
        );
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
        assert!(
            d.contains("gap too wide to run"),
            "unexpected diagnosis: {d}"
        );
        assert!(
            d.contains("6.00"),
            "the sprint distance belongs in the answer: {d}"
        );
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
        let climb = simulate_wall_climb(
            &lv,
            &platforms,
            6.0 - PHYSICS.player_w,
            6,
            TakeoffDir::Right,
        );
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

    // A pool eight tiles wide between two banks at the same height and **six tiles deep**. No
    // jump crosses eight tiles, and the depth is the point: a jump that falls in needs 300 ticks
    // to reach the floor (a 0.02-tile sink), which is past MAX_TRAJECTORY_TICKS, so the jump
    // sweep cannot discover the crossing by accident — the pool is a route only if a body
    // swims it at the surface and breaches out of the far lip.
    const POOL: &[&str] = &[
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "######~~~~~~~~######",
        "######~~~~~~~~######",
        "######~~~~~~~~######",
        "######~~~~~~~~######",
        "######~~~~~~~~######",
        "######~~~~~~~~######",
        "####################",
    ];

    // A ledge four rows above the ground that is reached **only** by a hanging vine in open
    // air — no wall beside it for a wall climb, and far too tall for a jump.
    const VINE: &[&str] = &[
        "................",
        ".....V..........",
        ".....V######....",
        ".....V..........",
        ".....V..........",
        ".....V..........",
        "################",
        "################",
    ];

    #[test]
    fn a_wide_pool_is_a_crossing_and_not_a_wall() {
        let lv = level("pool", POOL, (1, 4), &[("goal", 18, 4)]);
        let with = report(&lv);
        assert!(
            with.goal_reached,
            "a body can swim this pool, so the level is beatable: {:?}",
            with.diagnosis
        );
        assert!(with.culprit.is_none());

        // And the crossing really is the swim: take the script away and the same terrain is a
        // six-tile gap, which no jump in this game crosses.
        let without = report_with(
            &lv,
            ReachOptions {
                swim: false,
                ..ReachOptions::default()
            },
        );
        assert!(!without.goal_reached);
        assert!(
            without.diagnosis.is_some(),
            "an unbeatable level owes the designer a reason"
        );
    }

    #[test]
    fn a_swim_trajectory_reaches_the_far_bank() {
        let lv = level("pool", POOL, (1, 4), &[("goal", 18, 4)]);
        let platforms = extract_platforms(&lv);
        let far = platforms
            .iter()
            .find(|p| p.surface_row == 5 && p.x0 > 6)
            .expect("the far bank is a platform")
            .id;
        for dive in [false, true] {
            let swim = simulate_swim(&lv, &platforms, 5.0, 5, TakeoffDir::Right, dive);
            assert_eq!(
                swim.landed_on,
                Some(far),
                "the swim (dive = {dive}) should land on the far bank: {swim:?}"
            );
        }
    }

    #[test]
    fn a_hanging_vine_is_a_route_a_jump_is_not() {
        let lv = level("vine", VINE, (5, 5), &[("goal", 11, 1)]);
        let with = report(&lv);
        assert!(
            with.goal_reached,
            "the vine climbs to the ledge, so the level is beatable: {:?}",
            with.diagnosis
        );

        // Switch the vine off and the level is a four-tile wall with nothing beside it: the
        // exact false negative the owner hit when the oracle knew only jumps and walls. It is a
        // real control now because a jump that catches the vine is not credited to the jump
        // sweep — the vine script owns that landing.
        let without = report_with(
            &lv,
            ReachOptions {
                vine: false,
                ..ReachOptions::default()
            },
        );
        assert!(!without.goal_reached);
        let d = without.diagnosis.clone().expect("a diagnosis");
        assert!(d.contains("wall too tall"), "unexpected diagnosis: {d}");
    }

    #[test]
    fn a_vine_climb_reaches_the_ledge_it_hangs_beside() {
        let lv = level("vine", VINE, (5, 5), &[("goal", 11, 1)]);
        let platforms = extract_platforms(&lv);
        let ledge = platforms
            .iter()
            .find(|p| p.surface_row == 2 && p.contains_col(7))
            .expect("the ledge is a platform")
            .id;
        let x = 5.0 + (1.0 - PHYSICS.player_w) * 0.5;
        let climb = simulate_vine_climb(&lv, &platforms, x, 6, TakeoffDir::Right);
        assert_eq!(
            climb.landed_on,
            Some(ledge),
            "the climb should end on the ledge: {climb:?}"
        );
        assert!(climb.ticks > 30, "a four-tile climb is not instantaneous");
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
