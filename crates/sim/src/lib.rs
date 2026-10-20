//! `sim` — the deterministic core (development_plan.md §4, §5, §6).
//!
//! Hard boundary rules, enforced by review rather than by the type system:
//!
//! 1. No `std::time`, no threads, no `println!` inside [`Sim::step`], no `HashMap`
//!    in hot paths (SipHash has a per-process random seed → nondeterminism). The crate's
//!    one use of threads is the oracle's take-off sweep (the private `edges_from_all` in
//!    [`reach`]): offline work, a pure function of the terrain, run on a scoped pool and
//!    assembled in job order, so it cannot change what a run computes — only how long the
//!    answer takes to arrive. `a_parallel_sweep_answers_exactly_what_a_sequential_one_does`
//!    is what keeps that claim true rather than tidy.
//! 2. Everything the sim needs crosses the boundary as plain data:
//!    `&Level`, `AgentAction`, `seed: u64`.
//! 3. Rendering only ever borrows `&Sim`.
//!
//! A level's terrain is also asked questions that are expensive and pure — the move-graph
//! route table ([`reach::goal_routes`]) and the oracle's verdict ([`reach::report`]) — so both
//! are cached beside the level file, keyed by its content hash and the two behaviour versions
//! of the oracle that produced them (`route_cache`, private: nothing outside the crate asks
//! where an answer was kept, only for the answer). It is a derived artifact of what the level
//! already says, so it cannot change what a run computes — only how long the answer takes to
//! arrive, which for the second process to open a level is nothing at all.
//!
//! Version trio (§8): [`SIM_VERSION`], [`sensors::VERSION`], and
//! `evo::fitness::VERSION`. All three are stamped into every champion file.

pub mod hash;
pub mod level;
pub mod physics;
pub mod plan;
pub mod practice;
pub mod reach;
pub mod rng;
pub mod sensors;

/// The derived cache beside a level file.
mod route_cache;

use crate::physics::tile_span;

pub use crate::hash::StateHasher;
pub use crate::level::{FlyerSpec, Level, PlatformSpec, Rect, Scenery, Tile, TurretSpec};
pub use crate::physics::{
    AgentAction, Flyer, LavaBall, Mover, Physics, PlayerState, Shot, Turret, Walker, WalkerMode,
    PHYSICS,
};
pub use crate::rng::{common_episode_seed, episode_seed, rng_from_seed, Rng};

/// Sim behaviour version. Bump on *any* change to physics, tile semantics, or
/// episode termination — champions carrying an older value must be refused.
pub const SIM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Fixed timestep (§5.1). Headless training steps in a plain loop; only visible
/// realtime play needs an accumulator.
pub const DT: f32 = 1.0 / 60.0;

/// One tile is one world unit. Keeps the physics bible readable.
pub const TILE: f32 = 1.0;

/// Physics ticks between network queries: the **granularity of every decision the agent
/// makes**.
///
/// The sim owns this number rather than the trainer, because it is a physical fact about the
/// agent rather than a training knob. One action is held for this many ticks, so a block is
/// the atom of the agent's control — the shortest thing it can ask for, and the unit in which
/// it asks for everything longer. That is why [`physics::Physics::min_jump_hold`] is exactly
/// one of these: a jump held for a single block is the shortest jump the sim has, and each
/// further block of hold adds height, so jump *length* is a ladder with a rung per decision
/// spent. `evo` re-exports the value as its `--action-repeat` default, and
/// [`sensors::VERSION`] covers a change to it (§8).
pub const DECISION_TICKS: u32 = 4;

/// Fitness is scored per "pixel" of new rightmost ground covered (§7.4.1).
pub const PIXELS_PER_TILE: f32 = 16.0;

/// Episodes are declared stuck after this many frames with no new rightmost x
/// (§7.4.5). Without it, campers burn the whole compute budget standing still.
pub const NO_PROGRESS_FRAMES: u32 = 600;

/// Episode cap = `level.width * FRAMES_PER_TILE` (§7.4.6).
pub const FRAMES_PER_TILE: u32 = 40;

/// The episode cap for `level`: `max(width, height) × FRAMES_PER_TILE` (§7.4.6).
///
/// The *longer* side, not the width. A budget proportional to width alone quietly assumed
/// every level is a side-scroller: a 10x22 tower got 400 frames to climb 18 rows, which is
/// less than the 600-frame anti-camper timer and about seven jumps' worth of airtime. Every
/// shipped level is wider than it is tall, so this leaves all of them exactly as they were.
///
/// One function rather than the expression written twice, because two consumers now need the
/// same number: the sim stops an episode at it, and the scoring measures how much of it a
/// winning episode left unspent ([`EpisodeStats::budget_frames`]). A cap the two disagreed
/// about would grade a fast win as a slow one, and the disagreement would be invisible in
/// both.
pub fn frame_budget(level: &Level) -> u32 {
    level.width.max(level.height).max(1) as u32 * FRAMES_PER_TILE
}

/// A count of frames written as a **time**, the one way this project writes one.
///
/// The unit is the frame, because that is the unit everything else counts in: [`DT`] of a
/// second each, so `frames` is the whole of what a clock is and no caller has to know about a
/// wall-clock. Both front-ends and the champion metadata go through here rather than through
/// their own arithmetic, for the reason [`frame_budget`] is one function: a window that said
/// `8.4s` beside a terminal that said `504 frames` for the same episode would be two answers to
/// one question, and the number a viewer races has to be the number the AI was scored on.
///
/// Two shapes, deliberately. Under a minute it is hundredths of a second — `0.98s`, `8.42s` —
/// which is the resolution a race needs, since one frame is 0.017 s and two times that differ by
/// a frame have to be tellable apart. At a minute and up it is `m:ss.hh`, because `97.35s` is a
/// number a reader has to convert; no shipped level is anywhere near it (a full-width level's
/// budget is 40 s), so the second shape exists for the level someone draws tomorrow rather than
/// for one of ours.
pub fn format_time(frames: u32) -> String {
    let seconds = frames as f64 * DT as f64;
    if seconds < 60.0 {
        format!("{seconds:.2}s")
    } else {
        let minutes = (seconds / 60.0).floor();
        format!("{}:{:05.2}", minutes as u64, seconds - minutes * 60.0)
    }
}

/// What ended a dying episode. Diagnostics only: the score and the trajectory do not read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathCause {
    Spike,
    /// A lava cell or a ball from one. The ball is folded in because both are the same tile's
    /// attack and a reader asking "what killed me" wants the pool in either case.
    Lava,
    Shot,
    Walker,
    Flyer,
    Pit,
}

impl DeathCause {
    pub fn as_str(self) -> &'static str {
        match self {
            DeathCause::Spike => "spike",
            DeathCause::Lava => "lava",
            DeathCause::Shot => "turret shot",
            DeathCause::Walker => "walker",
            DeathCause::Flyer => "flyer",
            DeathCause::Pit => "fell out",
        }
    }
}

/// How an episode finished. `Running` means "not finished yet".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Running,
    /// Touched the goal flag.
    Won,
    /// Spike contact, walker side-hit, or fell into a pit.
    Died,
    /// Hit the frame cap set by [`FRAMES_PER_TILE`].
    TimedOut,
    /// Hit [`NO_PROGRESS_FRAMES`] without advancing.
    Stuck,
}

impl Outcome {
    pub fn is_finished(self) -> bool {
        self != Outcome::Running
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Running => "running",
            Outcome::Won => "won",
            Outcome::Died => "died",
            Outcome::TimedOut => "timed-out",
            Outcome::Stuck => "stuck",
        }
    }
}

/// Raw, unweighted episode measurements. Scoring lives in `evo::fitness` so the
/// fitness function can be versioned independently of the sim.
#[derive(Clone, Debug, PartialEq)]
pub struct EpisodeStats {
    /// Frames in which the episode was actually simulated.
    pub frames: u32,
    /// The frame cap this episode ran under ([`frame_budget`] for the level it was played on).
    ///
    /// Constant for the episode, and carried here rather than re-derived by whoever reads the
    /// stats because it is the *denominator* of the one reading that needs it: how much of the
    /// budget the body left unspent, which is what finishing quickly means in a game whose
    /// clock is its tick counter (`evo::fitness::SPEED_BONUS`). A consumer that recomputed the
    /// cap itself could disagree with the sim about when an episode ends, and a fast win would
    /// score as a slow one.
    pub budget_frames: u32,
    /// How much closer to the flag the body has ever been than it started, in pixels
    /// (§7.4.1).
    ///
    /// **Direction-agnostic since 0.5.0**, and that was a bug fix rather than a tidy-up.
    /// This used to be "new rightmost x", which quietly defined the game as a left-to-right
    /// side-scroller: on a level whose goal is *up* — a tower, a shaft, a climb — walking
    /// along the bottom scored better than climbing, and the episode was declared stuck
    /// while the body was on its way up. Distance-to-the-flag works in every direction,
    /// including diagonally, and on a rightward level it is the same signal it always was.
    pub progress_pixels: f32,
    /// The smallest distance to the flag the body has ever reached, in tiles. The running
    /// minimum that makes [`EpisodeStats::progress_pixels`] monotone.
    pub closest_approach: f32,
    /// Distance from the spawn to the flag, in pixels: the denominator that turns
    /// [`EpisodeStats::progress_pixels`] into a fraction of the level.
    ///
    /// Infinite for a level with no flag at all — the state a document is in between the
    /// editor's eraser and the next placement. Nothing divides by it without checking, so
    /// the arithmetic stays finite; the readouts that turn it into a percentage simply show
    /// no progress, which is what a level with nothing to reach is worth.
    pub goal_distance_pixels: f32,
    /// Rightmost x ever reached, in world units.
    ///
    /// Reported, not scored, since 0.5.0: it is what a watcher reads to see how far along a
    /// side-scroller the body got, but it is no longer the objective — `progress_pixels` is.
    pub best_x: f32,
    /// Consecutive frames with no new rightmost x.
    pub no_progress_frames: u32,
    /// Walkers stomped.
    pub stomps: u32,
    /// Longest run of stomps without touching the ground (WP13, reporting only).
    pub max_stomp_chain: u32,
    /// Wall jumps taken (WP13, reporting only).
    pub wall_jumps: u32,
    /// Spring-pad launches taken (WP13, reporting only).
    pub springs: u32,
    /// Brittle tiles that burned while the body was standing on them (WP13, reporting only).
    pub crumble_burned: u32,
    /// Longest airborne run, in ticks (WP13, reporting only).
    pub max_air_ticks: u32,
    /// Furthest horizontal distance from the take-off point reached in one airborne run, in
    /// pixels (WP14 reporting). A ground-free crossing is the technique this measures.
    pub max_air_distance: f32,
    /// Fastest body speed reached, tiles/tick (WP13, reporting only).
    pub peak_speed: f32,
    /// Jumps taken: ground jumps, buffered ones and wall jumps alike.
    ///
    /// Reported because it is the statistic that makes "the agent jumps endlessly"
    /// visible rather than a feeling, and scored (lightly, negatively) because a hop that
    /// buys no progress should not be free. See `evo::fitness::JUMP_PENALTY`.
    pub jumps: u32,
    /// Distinct safe landing zones the body has **stood on** that it could only have
    /// reached by crossing a hazard or a hole (scored in fitness v4, see
    /// `evo::fitness::CHECKPOINT_BONUS`).
    ///
    /// The companion to `jumps`: `jumps` is whether the body is trying, this is what
    /// trying has bought. Counted per *zone* ([`Level::landing_zone`]) rather than per
    /// tile, and only while the body is on the ground — so it cannot be collected by
    /// sailing over a hazard and dying beyond it, and a single zone pays once however many
    /// times the body walks back onto it.
    pub checkpoints: u32,
    /// Route crossings closed from the spawn stand toward the flag, measured only when
    /// grounded on a route stand (`spawn_hops - best_hops_reached`, in `0..=spawn_hops`).
    ///
    /// **Monotone along zig-zag routes where straight-line Euclidean distance reverses.**
    /// Every canonical crossing onto a stand fewer hops from the flag increments this
    /// count, giving evolution a true geodesic gradient up winding towers.
    pub route_hops_closed: u32,
    /// Best (deepest along the route) stand index the body has **stood on** during the
    /// episode, or `None` if it has not yet stood on a route stand.
    pub best_stand: Option<usize>,
    /// Distinct moving-platform decks the body has stood on, in first-arrival order. Scored in
    /// fitness v9: a shuttle crossing's first step is a landing, and the frontier cannot see it.
    pub decks: u32,
    /// Where the agent died, if it did.
    pub death_pos: Option<(f32, f32)>,
}

impl EpisodeStats {
    fn new(player: &PlayerState, level: &Level) -> Self {
        let approach = goal_approach(player, level);
        let spawn_stand = spawn_route_stand(level);
        EpisodeStats {
            frames: 0,
            budget_frames: frame_budget(level),
            progress_pixels: 0.0,
            closest_approach: approach,
            goal_distance_pixels: approach * PIXELS_PER_TILE,
            best_x: player.x,
            no_progress_frames: 0,
            stomps: 0,
            max_stomp_chain: 0,
            wall_jumps: 0,
            springs: 0,
            crumble_burned: 0,
            max_air_ticks: 0,
            max_air_distance: 0.0,
            peak_speed: 0.0,
            jumps: 0,
            checkpoints: 0,
            route_hops_closed: 0,
            best_stand: spawn_stand,
            decks: 0,
            death_pos: None,
        }
    }
}

/// **A plan as a sequence of positions (WP5).**
///
/// The planner's output is a list of block actions; this is the same route replayed into the
/// body positions it visits at each decision, plus the flag at the end. A guide is attached to a
/// training episode and never to a physics state: it changes no collision, no timing and no hash,
/// and a sim without one behaves exactly as it did before the structure existed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanGuide {
    /// Body centres along the planned route, in visit order, ending at the flag. The first point
    /// is the spawn, so a body that has moved nowhere reads one waypoint reached.
    pub points: Vec<(f32, f32)>,
    /// How close the body must be, in world units, to count the next waypoint as reached.
    pub radius: f32,
}

impl PlanGuide {
    /// The fraction of the plan's waypoints an episode reached, in `0..=1`.
    pub fn fraction(&self, reached: usize) -> f32 {
        if self.points.is_empty() {
            return 0.0;
        }
        reached.min(self.points.len()) as f32 / self.points.len() as f32
    }
}

/// The route stand index of the level's spawn point, when the spawn rests on a standable zone.
/// The route stand the level's spawn stands on, if it stands on one.
///
/// Public because *two* callers need the same answer and they must not drift: the sim uses it to
/// seed an episode's route progress (`hops_to_flag` from the spawn, which every later crossing is
/// measured against), and `evo` uses it to ask what a full route even is — the replay's length and
/// the frontier curriculum's starting stand. The three-tile window (exact column, then its
/// neighbours) is the same reading `Level::checkpoint_zone` makes of a body's feet, and it exists
/// because a spawn authored on the seam between two zones should not count as off-route.
pub fn spawn_route_stand(level: &Level) -> Option<usize> {
    let surface = Level::tile_y(level.spawn.y + level.spawn.h + 0.05);
    let col = Level::tile_x(level.spawn.x + level.spawn.w * 0.5);
    level
        .routes()
        .zone_at(col, surface)
        .or_else(|| level.routes().zone_at(col - 1, surface))
        .or_else(|| level.routes().zone_at(col + 1, surface))
}

/// Distance from a body to the level's goal, in tiles: zero when they overlap.
///
/// An AABB-to-AABB distance rather than a centre-to-centre one, so a wide flag on a ledge
/// does not read as "further away" than a narrow one at the same spot. A level has one flag
/// (`Level::goals`), and a level with *none* — a document mid-edit, not something a file can
/// describe — has no distance to close: the fold's `INFINITY` identity comes back, and
/// [`EpisodeStats::goal_distance_pixels`] documents what the readers do with it.
fn goal_approach(player: &PlayerState, level: &Level) -> f32 {
    let a = player.aabb();
    level
        .goals
        .iter()
        .map(|g| {
            let dx = (g.x - a.right()).max(a.x - g.right()).max(0.0);
            let dy = (g.y - a.bottom()).max(a.y - g.bottom()).max(0.0);
            (dx * dx + dy * dy).sqrt()
        })
        .fold(f32::INFINITY, f32::min)
}

/// Caller-owned buffers for [`Sim::observation_into`]: the mover rectangles, velocities and
/// phases a scene is built from. Kept outside the `Sim` so an evaluator running many episodes
/// reuses one set of allocations across all of them; the contents are rebuilt on every call
/// and mean nothing between calls.
#[derive(Clone, Debug, Default)]
pub struct ObservationScratch {
    movers: Vec<Rect>,
    mover_vel: Vec<(f32, f32)>,
    mover_phase: Vec<f32>,
}

impl ObservationScratch {
    pub fn new() -> Self {
        Self::default()
    }
}

/// One deterministically simulated episode.
///
/// Construct with [`Sim::new`], then drive with [`Sim::step`]. The full state
/// hashes to a `u64` via [`Sim::state_hash`]; the determinism test pins that.
#[derive(Clone, Debug)]
pub struct Sim {
    level: Level,
    player: PlayerState,
    walkers: Vec<Walker>,
    /// Flyers, in file order.
    flyers: Vec<Flyer>,
    /// Turrets, in file order.
    turrets: Vec<Turret>,
    /// Shots in flight.
    shots: Vec<Shot>,
    /// Lava balls in flight, oldest first.
    lava_balls: Vec<LavaBall>,
    /// Every lava cell open to the air, in row-major order: the vents a ball is spat from.
    ///
    /// Derived from the level once rather than stored as state — a vent is a fact about the
    /// terrain, so two runs of one level cannot disagree about where the fountains are. What is
    /// state is the balls, and those are hashed.
    lava_vents: Vec<(i32, i32)>,
    /// Moving platforms. Their *positions* are a function of the frame count; only which one
    /// the body is riding is state, and that is [`Sim::riding`].
    movers: Vec<Mover>,
    /// The mover the body stood on last frame, so this frame's motion can carry it.
    riding: Option<usize>,
    /// Deck ids the body has stood on at least once, in first-arrival order.
    ///
    /// A ride is a route hop and a landing, so it is worth crediting separately from a hazard
    /// checkpoint: on a level whose crossing *is* a shuttle, arrival on the deck is the step
    /// the frontier cannot see until the whole crossing is already made.
    decks_stood: Vec<usize>,
    /// Consecutive ticks on each side of a press: what a jump's timing is actually a function of.
    air_ticks: u32,
    ground_ticks: u32,
    /// Current stomp chain. Reset by a grounded frame that did not stomp (WP13 reporting).
    stomp_chain: u32,
    /// Where the current airborne run started, for [`EpisodeStats::max_air_distance`].
    air_start_x: f32,
    /// What ended a dying episode, for the `why` read. Not part of the score and not hashed:
    /// two trajectories that reach the same state are the same episode whatever killed them.
    death_cause: Option<DeathCause>,
    stats: EpisodeStats,
    outcome: Outcome,
    rng: Rng,
    /// Checkpoint zones already credited, as `(surface_row, zone_start)`. A linear scan of
    /// a list whose length is the level's hazard count, so a Vec is the right shape here —
    /// the hot path is the `last_stand` guard below, not this.
    credited: Vec<(i32, i32)>,
    /// The standing tile the last credit check looked at. A body resting on one tile is the
    /// common case by far, and this is what keeps it from re-deriving its landing zone
    /// (a scan back to the start of the run) on every one of those frames.
    last_stand: (i32, i32),
    /// The route stand index the body is currently standing on (or last stood on), used by
    /// the waypoint sensor (`sensors::observe_from_stand`) and geodesic progress tracking.
    current_route_stand: Option<usize>,
    /// Optional plan-proximity guide (WP5). Never part of the physics or the state hash.
    plan_guide: Option<PlanGuide>,
    /// Waypoints of [`Sim::plan_guide`] reached so far, monotone.
    plan_reached: usize,
    /// Route hops from the spawn stand to the flag, if the spawn is on a route to the flag.
    spawn_hops: Option<u32>,
    /// Minimum `hops_to_flag` reached on a grounded stand so far in this episode.
    min_hops_reached: Option<u32>,
    /// Brittle tiles whose fuse is burning: `((tile_x, tile_y), ticks remaining)`.
    ///
    /// A short `Vec` for the same reason [`Sim::credited`] is one — a level lights a handful per
    /// episode — and searched linearly, which is cheaper than the hashing a set would cost on the
    /// one frame in a hundred that has anything in it.
    crumbles: Vec<((i32, i32), u32)>,
    /// Every brittle tile that has left the level this episode, oldest first.
    ///
    /// The sim does not need this to *run* — the level's own tile array is the truth, and it is
    /// what the renderer draws — but it is what makes [`Sim::state_hash`] a complete statement of
    /// the terrain a body has changed: the tiles that are gone cannot be recovered from a level
    /// that was already missing them when the episode started, and two runs can only agree on
    /// this list if they agreed on every fuse that lit.
    broken: Vec<(i32, i32)>,
}

impl Sim {
    /// Start an episode. Same `level` + `seed` ⇒ identical trajectory.
    ///
    /// The episode RNG is consumed here, once per walker, to draw that walker's
    /// patrol phase ([`Walker::spawn_spread`]). Drawing at construction rather
    /// than during `step` keeps the RNG stream position a pure function of the
    /// walker count, so the state hash stays comparable across frames.
    pub fn new(level: &Level, seed: u64) -> Self {
        let mut rng = rng_from_seed(seed);
        let mut walkers: Vec<Walker> = level
            .walkers
            .iter()
            .enumerate()
            .map(|(i, r)| Walker::spawn_spread(i as u32, *r, level, &PHYSICS, &mut rng))
            .collect();
        // **Chasers are appended after the patrols, and draw no RNG.** Two reasons that matter:
        // a walker level's enemy-vector tie-break is walker index, so adding chasers must not
        // renumber the patrols; and a level that has none must consume the identical RNG stream
        // it did before this pass, or every recorded walker episode in the pack would change.
        for r in &level.chasers {
            let id = walkers.len() as u32;
            walkers.push(Walker::chaser(id, *r, &PHYSICS));
        }
        let flyers = level
            .flyers
            .iter()
            .enumerate()
            .map(|(i, spec)| Flyer::spawn_spread(i as u32, *spec, &PHYSICS, &mut rng))
            .collect();
        let turrets = level
            .turrets
            .iter()
            .enumerate()
            .map(|(i, spec)| Turret::new(i as u32, *spec, &PHYSICS))
            .collect();
        let movers = level
            .platforms
            .iter()
            .enumerate()
            .map(|(i, spec)| Mover {
                id: i as u32,
                home: spec.rect,
                span: spec.span,
                vertical: spec.vertical,
                dir: spec.dir,
                // Where the clear part of the travel ends. A platform authored through a
                // wall/ceiling turns early rather than passing through it.
                limit: crate::physics::clear_travel(
                    spec.rect,
                    spec.span,
                    spec.vertical,
                    spec.dir,
                    level,
                ),
            })
            .collect();
        let spawn_player = level.spawn_player();
        let current_route_stand = spawn_route_stand(level);
        let spawn_hops = current_route_stand.and_then(|s| level.routes().hops_to_flag(s));
        Sim {
            player: spawn_player,
            walkers,
            flyers,
            turrets,
            shots: Vec::new(),
            lava_balls: Vec::new(),
            lava_vents: level.lava_vents(),
            movers,
            riding: None,
            decks_stood: Vec::new(),
            air_ticks: 0,
            ground_ticks: 0,
            stomp_chain: 0,
            air_start_x: spawn_player.x,
            death_cause: None,
            stats: EpisodeStats::new(&spawn_player, level),
            level: level.clone(),
            outcome: Outcome::Running,
            rng,
            credited: Vec::new(),
            // No tile is ever at a negative row, so this cannot match a real standing tile
            // and the first grounded frame always takes the slow path once.
            last_stand: (i32::MIN, i32::MIN),
            current_route_stand,
            plan_guide: None,
            plan_reached: 0,
            spawn_hops,
            min_hops_reached: spawn_hops,
            crumbles: Vec::new(),
            broken: Vec::new(),
        }
    }

    pub fn level(&self) -> &Level {
        &self.level
    }

    pub fn player(&self) -> &PlayerState {
        &self.player
    }

    pub fn walkers(&self) -> &[Walker] {
        &self.walkers
    }

    pub fn flyers(&self) -> &[Flyer] {
        &self.flyers
    }

    pub fn turrets(&self) -> &[Turret] {
        &self.turrets
    }

    pub fn shots(&self) -> &[Shot] {
        &self.shots
    }

    /// The lava balls in flight this frame.
    pub fn lava_balls(&self) -> &[LavaBall] {
        &self.lava_balls
    }

    /// The lava cells that are spitting: one per open-air lava cell in the level.
    pub fn lava_vents(&self) -> &[(i32, i32)] {
        &self.lava_vents
    }

    /// The moving platforms, as the boxes they occupy **this frame**.
    ///
    /// A renderer or a test asks this rather than the specs, because a platform's position is
    /// a fact about a frame and the specs only carry the travel.
    pub fn platforms(&self) -> Vec<Rect> {
        self.movers
            .iter()
            .map(|m| m.rect_at(self.stats.frames, &PHYSICS))
            .collect()
    }

    pub fn stats(&self) -> &EpisodeStats {
        &self.stats
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// What ended a dying episode, when the read is asked for after the fact.
    pub fn death_cause(&self) -> Option<DeathCause> {
        self.death_cause
    }

    /// The route stand the body is on (or last stood on), for the waypoint sensor and a
    /// planner's `which rung is this` question.
    pub fn route_stand(&self) -> Option<usize> {
        self.current_route_stand
    }

    /// Attach a plan-proximity guide (WP5). The guide is reporting/selection data only: it does
    /// not touch the body, the world or [`Sim::state_hash`].
    pub fn set_plan_guide(&mut self, guide: PlanGuide) {
        self.plan_reached = 0;
        self.plan_guide = Some(guide);
        self.advance_plan_guide();
    }

    /// `(waypoints reached, waypoints in the guide)`, the numerator and denominator of the plan
    /// fraction. `(0, 0)` exactly when no guide is attached, which is what lets a caller tell
    /// "no plan" from "a plan it has not moved on".
    pub fn plan_progress(&self) -> (usize, usize) {
        match &self.plan_guide {
            Some(guide) => (self.plan_reached, guide.points.len()),
            None => (0, 0),
        }
    }

    /// **A snapshot restarted as a fresh episode (WP8).**
    ///
    /// The world — body, enemies, movers, world clock — is exactly the snapshot, so a policy
    /// started here meets the phase it was archived in. What resets is the *episode accounting*:
    /// progress and closest approach are measured from this state, route/deck/checkpoint credit
    /// starts again, and the outcome is running. `stats.frames` is deliberately **kept**, because
    /// movers and turrets are functions of the world clock; a snapshot that reset it to zero would
    /// be a different world wearing the same geometry.
    ///
    /// The returned sim is a normal episode from here on, and a result produced from it is
    /// replayable only from the snapshot — which is why this state never crowns a record without a
    /// spawn-started validation.
    pub fn restart_from_snapshot(&self) -> Sim {
        let mut sim = self.clone();
        let mut stats = EpisodeStats::new(&sim.player, &sim.level);
        stats.frames = sim.stats.frames;
        stats.budget_frames = frame_budget(&sim.level);
        stats.best_stand = sim.current_route_stand;
        stats.best_x = sim.player.x;
        sim.stats = stats;
        sim.outcome = Outcome::Running;
        sim.death_cause = None;
        sim.stomp_chain = 0;
        sim.decks_stood.clear();
        sim.credited.clear();
        sim.spawn_hops = sim
            .current_route_stand
            .and_then(|s| sim.level.routes().hops_to_flag(s));
        sim.min_hops_reached = sim.spawn_hops;
        sim.no_progress_frames_reset();
        let surface = Level::tile_y(sim.player.y + sim.player.h + 0.05);
        let col = Level::tile_x(sim.player.x + sim.player.w * 0.5);
        sim.last_stand = (surface, col);
        sim.plan_guide = None;
        sim.plan_reached = 0;
        sim
    }

    /// Reset the anti-camper timer after a snapshot restart. Kept as a method rather than a
    /// direct field write because `no_progress_frames` is the one stat the termination rule reads
    /// on the very next tick, and a reset that forgot it would end a snapshot episode as `Stuck`
    /// before it moved.
    fn no_progress_frames_reset(&mut self) {
        self.stats.no_progress_frames = 0;
    }

    /// Move the guide's cursor past every point the body has now reached. Called once per tick;
    /// the cursor is monotone, so a body that backtracks cannot lose plan credit it earned.
    fn advance_plan_guide(&mut self) {
        let Some(guide) = &self.plan_guide else {
            return;
        };
        let (cx, cy) = self.player.aabb().center();
        let radius2 = guide.radius.max(0.0).powi(2);
        while let Some(&(px, py)) = guide.points.get(self.plan_reached) {
            let d2 = (px - cx).powi(2) + (py - cy).powi(2);
            if d2 > radius2 {
                break;
            }
            self.plan_reached += 1;
        }
    }

    /// A copy of the whole episode state, for planning and frontier restarts. The sim is
    /// deterministic, so a snapshot plus an action sequence is an episode; nothing else is
    /// needed to fork a search or to restart a run at the state it stalled in.
    pub fn snapshot(&self) -> Sim {
        self.clone()
    }

    /// Advance exactly one physics tick (1/[`DT`] of a second).
    pub fn step(&mut self, action: AgentAction) {
        if self.outcome.is_finished() {
            return;
        }
        let ph = &PHYSICS;
        self.stats.frames += 1;
        let frame = self.stats.frames;

        // The level is alive whatever the player does: enemies move, turrets fire, shots fly.
        // A chaser is handed the body's position; a patrol ignores it.
        let target_x = self.player.x;
        for w in self.walkers.iter_mut() {
            w.tick_against(&self.level, Some(target_x), ph);
        }
        for f in self.flyers.iter_mut() {
            f.tick(&self.level, ph);
        }
        for t in self.turrets.iter_mut() {
            if t.tick(ph) {
                self.shots.push(Shot::from_turret(t, ph));
            }
        }
        for s in self.shots.iter_mut() {
            s.tick(&self.level, ph);
        }
        self.shots.retain(|s| s.alive);

        // ---- lava: the pool throws, gravity brings it back -----------------
        //
        // Every vent fires on its own phase of one period, derived from its index rather than
        // drawn from the episode RNG — a fountain is a property of the level, like a walker's
        // patrol path, and two runs of one seed must see the same rhythm. The phase stride
        // (`37` against a period of 150) is coprime with the period, so a row of vents fires
        // in a rolling order and a body reading the row has a gap to move through.
        for (i, vent) in self.lava_vents.iter().enumerate() {
            if LavaBall::fires(i, frame, ph) {
                self.lava_balls.push(LavaBall::spawn(*vent, i, ph));
            }
        }
        for b in self.lava_balls.iter_mut() {
            b.tick(&self.level, ph);
        }
        self.lava_balls.retain(|b| b.alive);

        // ---- moving platforms: the shelf carries its rider *before* the body moves ---
        //
        // The order is the whole of the jitter the first version had. Carrying *after* the
        // body's tick means gravity pulls the rider down from the shelf's old position and
        // then the landing snap fights it back up, once per frame, forever. Carrying before
        // means the body starts its tick on the deck as it is now, so there is nothing to
        // fight. The horizontal half is refused when it would put the body inside terrain:
        // the player's own x movement is blocked that way, and a shelf must not push a body
        // into a wall that the body cannot walk into.
        //
        // **The feet are recorded before the carry, not after it.** The landing test below is a
        // *crossing* — above the deck on one frame, at or below it on the next — and both halves
        // of that crossing have to be measured the same way. Measured after the carry, the
        // shelf's own motion was counted as the body's: a lift rising 0.0625 a tick moved the
        // feet by exactly that much before the test looked at them, so "where the feet were"
        // was already "where the shelf put them". The crossing then passed for the wrong reason
        // on a lift and could fail for the wrong reason anywhere else — a carried body refused
        // at a wall, or one a tolerance away from the deck, lost its ride for a frame and was
        // re-caught by the snap, which looks like a platform that grabs.
        let prev_bottom = self.player.y + self.player.h;
        for (i, m) in self.movers.iter().enumerate() {
            if self.riding != Some(i) {
                continue;
            }
            let (dx, dy) = m.delta_at(frame, ph);
            if dx != 0.0 {
                let before = self.player.x;
                self.player.carry(dx, 0.0);
                if crate::physics::rect_hits_opaque(&self.level, self.player.aabb()) {
                    self.player.x = before;
                }
            }
            if dy != 0.0 {
                self.player.carry(0.0, dy);
            }
        }

        let touched = self.player.tick(&self.level, ph, action);
        if touched.jumped {
            self.stats.jumps += 1;
        }
        if touched.wall_jumped {
            self.stats.wall_jumps += 1;
        }
        if self.player.grounded {
            self.ground_ticks = self.ground_ticks.saturating_add(1);
            self.air_ticks = 0;
        } else {
            if self.air_ticks == 0 {
                self.air_start_x = self.player.x;
            }
            self.air_ticks = self.air_ticks.saturating_add(1);
            self.ground_ticks = 0;
            self.stats.max_air_ticks = self.stats.max_air_ticks.max(self.air_ticks);
            self.stats.max_air_distance = self
                .stats
                .max_air_distance
                .max((self.player.x - self.air_start_x).abs());
        }
        let speed = (self.player.vx * self.player.vx + self.player.vy * self.player.vy).sqrt();
        self.stats.peak_speed = self.stats.peak_speed.max(speed);

        // ---- springs: a pad throws a falling body back up -----------------
        // The rule itself — and the reasoning for every half of it — is `physics::spring_launch`,
        // beside the launch box and the height the throw produces. It is shared with the
        // reachability oracle rather than written here because the two must not disagree about how
        // far a move goes: the oracle runs the same call inside its canonical trajectories, which
        // is what makes a pad-only crossing something it can find (`REVIEW.md` §1.12).
        if crate::physics::spring_launch(&self.level, &mut self.player, ph) {
            self.stats.springs += 1;
        }

        // ---- landing on a mover: a crossing, not a position -----------------
        //
        // A mover is **ground from above and air from the side**, exactly like a one-way
        // platform. The first version's test — "the feet are above the deck" — was true for a
        // body *three tiles* above the shelf, so a falling body was teleported down onto it
        // from any height, and a body walking under one was yanked up through the deck. A
        // landing has to be a **crossing**: the feet were at or above the deck last frame and
        // are at or below it now. That is the same surface test the tile grid's own landing
        // does, and it is the difference between a platform and a lift-shaped magnet.
        let mut ride = None;
        for (i, m) in self.movers.iter().enumerate() {
            let rect = m.rect_at(frame, ph);
            let feet = self.player.y + self.player.h;
            let horiz = self.player.x + self.player.w > rect.x && self.player.x < rect.x + rect.w;
            let crossed =
                prev_bottom <= rect.y + ph.enemy_stomp_tol && feet >= rect.y - ph.enemy_stomp_tol;
            // A body that was *already* riding keeps riding while it is still over the deck and
            // still standing on it, and this is not a refinement — it is what makes the ride
            // continuous. The crossing test alone re-asks "did the feet just arrive?" on every
            // frame, so any frame where the answer slipped — the horizontal carry refused at a
            // wall, a lift that outran the tolerance, the one frame after a stoppped jump —
            // dropped the ride and left the body in the air a tick later, to be re-caught by the
            // snap. A rider is a *state*, and `self.riding` is where it is kept.
            let stayed = self.riding == Some(i) && (feet - rect.y).abs() <= ph.enemy_stomp_tol;
            if horiz && self.player.vy >= 0.0 && (crossed || stayed) {
                self.player.land_on(rect.y);
                ride = Some(i);
            }
        }
        self.riding = ride;
        if let Some(id) = ride {
            if !self.decks_stood.contains(&id) {
                self.decks_stood.push(id);
                self.stats.decks = self.decks_stood.len() as u32;
            }
        }

        // ---- hazards ------------------------------------------------------
        if touched.touched_spike {
            self.die(DeathCause::Spike);
            return;
        }
        // Lava kills twice: the pool a body steps into, and the ball it did not see coming.
        // Both are the same death, which is the point of the tile having two mechanics.
        if touched.touched_lava
            || self
                .lava_balls
                .iter()
                .any(|b| b.alive && self.player.aabb().overlaps(&b.aabb()))
        {
            self.die(DeathCause::Lava);
            return;
        }

        // ---- walkers: stomp vs side-hit ----------------------------------
        let mut stomped = false;
        for w in self.walkers.iter_mut() {
            if !w.alive || !self.player.aabb().overlaps(&w.aabb(ph)) {
                continue;
            }
            let descending = self.player.vy > 0.0;
            let above = prev_bottom <= w.y + ph.enemy_stomp_tol;
            if descending && above {
                w.alive = false;
                self.player.vy = -ph.jump_v0 * ph.stomp_bounce;
                self.player.grounded = false;
                stomped = true;
            } else {
                self.die(DeathCause::Walker);
                return;
            }
        }
        if stomped {
            self.stats.stomps += 1;
            self.stomp_chain = self.stomp_chain.saturating_add(1);
            self.stats.max_stomp_chain = self.stats.max_stomp_chain.max(self.stomp_chain);
        } else if self.player.grounded {
            // A stomp chain is consecutive, so touching ordinary ground ends it. A bounce from a
            // stomp leaves the body ungrounded, which is what keeps the chain alive.
            self.stomp_chain = 0;
        }

        // ---- flyers: the same stomp rule as a walker, one row up ------------
        for f in self.flyers.iter_mut() {
            if !f.alive || !self.player.aabb().overlaps(&f.aabb()) {
                continue;
            }
            let descending = self.player.vy > 0.0;
            let above = prev_bottom <= f.y + ph.enemy_stomp_tol;
            if descending && above {
                f.alive = false;
                self.player.vy = -ph.jump_v0 * ph.stomp_bounce;
                self.player.grounded = false;
                self.stats.stomps += 1;
                self.stomp_chain = self.stomp_chain.saturating_add(1);
                self.stats.max_stomp_chain = self.stats.max_stomp_chain.max(self.stomp_chain);
            } else {
                self.die(DeathCause::Flyer);
                return;
            }
        }

        // ---- turrets: a solid obstacle, not an enemy -----------------------
        //
        // A turret used to be stompable like a walker: coming down on one destroyed it and
        // bounced the body, and touching its side was a death. Both halves are gone. It is a
        // **block that happens to shoot**: the body stands on its lid, is stopped by its sides,
        // and is never killed by the box itself — the shot keeps its whole role as the row's
        // hazard, which is what the turret is for. The lid is not a stomp and does not count in
        // `stats.stomps`; it is the same surface a tile would be, and a body that walks off it
        // falls like any other.
        //
        // Resolved *after* the body's own tile pass, because the box is not in the grid: the
        // tile pass is what stops it at the floor and the walls, and this is a second solid
        // surface laid over the cells the turret occupies.
        self.resolve_turret_solidity(prev_bottom);

        // ---- shots: there is no stomping a bullet ----------------------------
        if self
            .shots
            .iter()
            .any(|s| s.alive && self.player.aabb().overlaps(&s.aabb()))
        {
            self.die(DeathCause::Shot);
            return;
        }

        // ---- goal --------------------------------------------------------
        if self
            .level
            .goals
            .iter()
            .any(|g| self.player.aabb().overlaps(g))
        {
            self.outcome = Outcome::Won;
            // **Winning is reaching the plan's last waypoint.** The guide's final point is the
            // flag's centre, and the flag is a trigger box, so a body can win while its centre is
            // still further than the guide's radius from that centre. This branch returns before
            // the guide cursor is advanced, so without this the planner's own route would read
            // 34 of 35 waypoints on a level it solved.
            if let Some(guide) = &self.plan_guide {
                self.plan_reached = guide.points.len();
            }
            self.finish();
            return;
        }

        // ---- pit ---------------------------------------------------------
        if self.player.y > self.level.height as f32 {
            self.die(DeathCause::Pit);
            return;
        }

        // ---- brittle ground: light the fuses, then burn them ----------------
        // A brittle tile is ordinary solid ground until a body *stands* on it, so the ignition
        // test is the same "which tiles are under the feet" question the checkpoint pass below
        // asks, and it is the standing that matters rather than the touch: a body that clips the
        // edge of a cracked tile on its way over a pit did not walk on it.
        //
        // **A walker's patrol does not light a fuse**, and that is a decision rather than an
        // oversight. A patrol is a machine walking the same three tiles for the whole episode: it
        // would demolish a level's floor without ever meeting the problem the level is built
        // around, and every cracked bridge in the pack would have to be drawn to survive a walker
        // as well as a player. Brittle ground is a trap for the body that has to *choose* to cross
        // it, which is the body the level is addressed to.
        //
        // Burn first, then light, and the order is the fuse's own definition: a tile lit on frame F
        // spends `CRUMBLE_TICKS` frames underfoot and is gone on frame `F + CRUMBLE_TICKS`. Lighting
        // first would spend the fuse's first frame on the very frame the body stepped down, which
        // makes the stated length of a fuse a number nobody can point at from outside the sim.
        //
        // Written through [`Level::burn_tile`] rather than through `set_tile`: the physics below
        // reads the live grid, so a hole the body has just opened is a hole it falls through,
        // while the route table the waypoint sensor steers by keeps describing the level as it
        // was authored. Rebuilding that table per broken tile was, at a sweep of milliseconds a
        // time, the single largest cost in training a level with brittle ground.
        if !self.crumbles.is_empty() {
            let mut i = 0;
            while i < self.crumbles.len() {
                let (tile, left) = self.crumbles[i];
                if left <= 1 {
                    self.crumbles.swap_remove(i);
                    self.level.burn_tile(tile.0, tile.1);
                    self.broken.push(tile);
                    // WP13: did the body ride this tile until it burned? A fuse that expired
                    // underfoot is the technique a crumble-surf is made of; a fuse the body
                    // outran is not credited.
                    if self.player.grounded {
                        let feet = Level::tile_y(self.player.y + self.player.h + 0.05);
                        let (c0, c1) = tile_span(self.player.x, self.player.w);
                        if tile.1 == feet && tile.0 >= c0 && tile.0 <= c1 {
                            self.stats.crumble_burned += 1;
                        }
                    }
                } else {
                    self.crumbles[i].1 = left - 1;
                    i += 1;
                }
            }
        }
        // Which brittle tiles the body is standing on this frame, and therefore which fuses are
        // lit. The tiles are read *after* the pass above, so a tile that left the level on this
        // frame is not immediately lit again by the body that was standing on it.
        if self.player.grounded {
            let row = Level::tile_y(self.player.y + self.player.h + 0.05);
            let (c0, c1) = tile_span(self.player.x, self.player.w);
            for tx in c0..=c1 {
                if self.level.get(tx, row).is_cracked()
                    && !self.crumbles.iter().any(|(tile, _)| *tile == (tx, row))
                {
                    self.crumbles
                        .push(((tx, row), crate::physics::CRUMBLE_TICKS));
                }
            }
        }

        // ---- checkpoints & geodesic route progress -----------------------
        // Credit the safe landing zones the body has walked onto. Grounded-only on
        // purpose: the term exists to pay for *arriving* somewhere, so a body that sails
        // over a hazard and dies in the air beyond it keeps the frontier it covered and
        // earns nothing here. See `Level::checkpoint_zone_from`.
        let mut route_advanced = false;
        if self.player.grounded {
            let surface = Level::tile_y(self.player.y + self.player.h + 0.05);
            let col = Level::tile_x(self.player.x + self.player.w * 0.5);
            if (surface, col) != self.last_stand {
                let from_col = if self.last_stand.0 == i32::MIN {
                    Level::tile_x(self.level.spawn.x + self.level.spawn.w * 0.5)
                } else {
                    self.last_stand.1
                };
                self.last_stand = (surface, col);
                if let Some(zone) = self.level.checkpoint_zone_from(col, surface, from_col) {
                    let id = (surface, zone);
                    if !self.credited.contains(&id) {
                        self.credited.push(id);
                        self.stats.checkpoints = self.credited.len() as u32;
                    }
                }
                let routes = self.level.routes();
                let stand = routes
                    .zone_at(col, surface)
                    .or_else(|| routes.zone_at(col - 1, surface))
                    .or_else(|| routes.zone_at(col + 1, surface));
                if let Some(s) = stand {
                    self.current_route_stand = Some(s);
                    if let Some(hops) = routes.hops_to_flag(s) {
                        if self.min_hops_reached.is_none_or(|m| hops < m) {
                            self.min_hops_reached = Some(hops);
                            self.stats.best_stand = Some(s);
                            if let Some(sh) = self.spawn_hops {
                                let closed = sh.saturating_sub(hops);
                                if closed > self.stats.route_hops_closed {
                                    self.stats.route_hops_closed = closed;
                                    route_advanced = true;
                                }
                            }
                        } else if self.stats.best_stand.is_none() {
                            self.stats.best_stand = Some(s);
                        }
                    } else if self.stats.best_stand.is_none() {
                        self.stats.best_stand = Some(s);
                    }
                }
            }
        }

        self.advance_plan_guide();

        // ---- progress / termination --------------------------------------
        // Progress is *how much closer to the flag than the spawn the body has ever been*,
        // so a level never starts "in credit" and the objective has no preferred direction: a
        // tower is climbed, a corridor is run, and neither is a special case (see
        // `EpisodeStats::progress_pixels` for what this replaced and why). Recomputing the
        // total from the running minimum rather than accumulating a delta keeps it exactly
        // monotone in floating point, so a route that leaves the flag and comes back cannot
        // lose ground already banked.
        let approach = goal_approach(&self.player, &self.level);
        if approach < self.stats.closest_approach {
            self.stats.closest_approach = approach;
            self.stats.progress_pixels =
                self.stats.goal_distance_pixels - approach * PIXELS_PER_TILE;
            self.stats.no_progress_frames = 0;
        } else if route_advanced || self.waiting_for_a_mover() {
            self.stats.no_progress_frames = 0;
        } else {
            self.stats.no_progress_frames += 1;
        }
        if self.player.x > self.stats.best_x {
            self.stats.best_x = self.player.x;
        }

        if self.stats.no_progress_frames >= NO_PROGRESS_FRAMES {
            self.outcome = Outcome::Stuck;
            self.finish();
        } else if self.stats.frames >= self.max_frames() {
            self.outcome = Outcome::TimedOut;
            self.finish();
        }
    }

    /// Frame cap for this level (§7.4.6), i.e. [`frame_budget`] of its terrain.
    pub fn max_frames(&self) -> u32 {
        frame_budget(&self.level)
    }

    /// Is the body waiting for a moving platform rather than camping?
    ///
    /// The anti-camper timer was written for a corridor, where standing still is always a
    /// mistake: the way on is to the right and a body that has stopped moving is not going to
    /// find it. A timed crossing breaks that premise. The correct move in front of a shuttle is
    /// often to **stand exactly where you are** until the deck arrives, and the old rule killed
    /// that body at frame 600 for doing the right thing — which is one of the reasons the pack's
    /// mover levels are hard for a policy that already knows it has to wait.
    ///
    /// So patience is paused in the two states where waiting is the world's fault rather than the
    /// body's: while it is riding a deck (the deck is moving it, even when its x does not fall),
    /// and while it is standing on ground a deck's own travel can reach at its own foot level.
    /// It is a pause, not an exemption: the episode's frame budget still ends a body that waits
    /// forever, and a deck that can never reach the body does not pause anything.
    fn waiting_for_a_mover(&self) -> bool {
        if self.riding.is_some() {
            return true;
        }
        if !self.player.grounded {
            return false;
        }
        let p = &self.player;
        let cx = p.x + p.w * 0.5;
        let feet = p.y + p.h;
        self.movers.iter().any(|m| {
            let span = m.limit.clamp(0.0, m.span.max(0.0));
            let reach = m.dir as f32 * span;
            let (x0, x1, y0, y1) = if m.vertical {
                (
                    m.home.x,
                    m.home.x + m.home.w,
                    m.home.y.min(m.home.y + reach),
                    m.home.y.max(m.home.y + reach) + m.home.h,
                )
            } else {
                (
                    m.home.x.min(m.home.x + reach),
                    m.home.x.max(m.home.x + reach) + m.home.w,
                    m.home.y,
                    m.home.y + m.home.h,
                )
            };
            // A deck whose travel passes over the body's column, at a height it could step onto
            // from where it stands. The two-tile window is a jump's own reach: a deck further
            // above is not a step this body is waiting for.
            cx >= x0 - 1.0 && cx <= x1 + 1.0 && feet >= y0 - 2.0 && feet <= y1 + 2.0
        })
    }

    /// **A turret is a solid box.** Stand on its lid; a side stops the body.
    ///
    /// The turret used to live in the enemy pass above: a stomp destroyed it (and bounced the
    /// body), and any other touch killed. It is furniture now — a barrel on a plinth that fires
    /// — so the two questions a solid tile answers are answered here, after the body's own tile
    /// pass has run: **from above** the lid is ground, and **from the side** the box is a wall.
    /// Neither is a stomp and neither is a death; the shot is the only hazard the turret owns.
    ///
    /// A landing is a *crossing*, exactly as on a moving platform: the feet were at or above
    /// the lid on the previous tick and are inside the box now. Without the crossing half, a
    /// body falling beside a turret would be snapped up onto it from a tile away.
    fn resolve_turret_solidity(&mut self, prev_bottom: f32) {
        let ph = &PHYSICS;
        for t in self.turrets.iter().filter(|t| t.alive) {
            let lid = Rect::new(t.x, t.y, t.w, t.h);
            let body = self.player.aabb();
            if !body.overlaps(&lid) {
                continue;
            }
            if self.player.vy >= 0.0 && prev_bottom <= lid.y + ph.enemy_stomp_tol {
                // `land_on` stops the fall, grounds the body and clears the jump cut, so the
                // next tick is a standing one rather than another fall into the lid.
                self.player.land_on(lid.y);
                continue;
            }
            // The side: put the body back out along the way it was travelling. Pushing the
            // other way would be a wall that teleports a body through itself, and a body with
            // no horizontal speed is pushed out the nearer face so it can never end up on the
            // wrong side of the turret after standing still against it.
            if self.player.vx > 0.0 {
                self.player.x = lid.x - self.player.w;
            } else if self.player.vx < 0.0 {
                self.player.x = lid.x + lid.w;
            } else if body.x + body.w * 0.5 < lid.x + lid.w * 0.5 {
                self.player.x = lid.x - self.player.w;
            } else {
                self.player.x = lid.x + lid.w;
            }
            self.player.vx = 0.0;
        }
    }

    fn die(&mut self, cause: DeathCause) {
        self.death_cause = Some(cause);
        self.outcome = Outcome::Died;
        self.stats.death_pos = Some((self.player.x, self.player.y));
        self.finish();
    }

    fn finish(&mut self) {
        debug_assert!(self.outcome.is_finished());
    }

    /// How close the body is to the flag *right now*, in tiles ([`goal_approach`]).
    ///
    /// The current reading, as opposed to [`EpisodeStats::closest_approach`] — the running
    /// minimum over the whole episode, which includes positions the body only passed through
    /// and never kept (the apex of a leap it then fell back from). Ask this one when the
    /// question is "where has the body actually *stood*": `evo::curriculum::stuck_point`
    /// tracks it over grounded frames to find the spot a stalled run should practice from.
    pub fn goal_approach(&self) -> f32 {
        goal_approach(&self.player, &self.level)
    }

    /// Observation vector for the agent policy (§7.2). See [`sensors`].
    ///
    /// Everything alive is handed over, because the enemy slots answer about the *nearest
    /// threat* and a flyer or a shot is one: an agent that could only see walkers would be
    /// blind to exactly the two enemies added to make height and distance dangerous.
    ///
    /// This allocating form is the convenience wrapper; the per-decision hot paths (episode
    /// evaluation, committee play) call [`Sim::observation_into`] with a reused
    /// [`ObservationScratch`] instead, because building the scene used to allocate three
    /// vectors per decision and a training run makes millions of decisions.
    pub fn observation(&self) -> [f32; sensors::N_INPUTS] {
        self.observation_into(&mut ObservationScratch::default())
    }

    /// The same observation, with the scene's mover vectors built in caller-owned scratch
    /// buffers whose capacity is reused across calls. Bit-for-bit the vector
    /// [`Sim::observation`] returns: the scratch changes where the intermediate vectors
    /// live, never what they contain (pinned by a test that compares the two over an
    /// episode on a level with movers).
    pub fn observation_into(
        &self,
        scratch: &mut ObservationScratch,
    ) -> [f32; sensors::N_INPUTS] {
        let frames = self.stats.frames;
        scratch.movers.clear();
        scratch
            .movers
            .extend(self.movers.iter().map(|m| m.rect_at(frames, &PHYSICS)));
        scratch.mover_vel.clear();
        scratch
            .mover_vel
            .extend(self.movers.iter().map(|m| m.delta_at(frames, &PHYSICS)));
        scratch.mover_phase.clear();
        scratch
            .mover_phase
            .extend(self.movers.iter().map(|m| m.phase_at(frames, &PHYSICS)));
        sensors::observe_scene(
            &self.level,
            &self.player,
            &sensors::Scene {
                walkers: &self.walkers,
                flyers: &self.flyers,
                shots: &self.shots,
                lava_balls: &self.lava_balls,
                movers: &scratch.movers,
                mover_vel: &scratch.mover_vel,
                mover_phase: &scratch.mover_phase,
                turrets: &self.turrets,
                vents: &self.lava_vents,
                air_ticks: self.air_ticks,
                ground_ticks: self.ground_ticks,
                frames: self.stats.frames,
            },
            &PHYSICS,
            self.current_route_stand,
        )
    }

    /// Canonical hash of the complete simulated state (§5.2, §5.3).
    ///
    /// Floats are hashed by their IEEE-754 bit pattern after canonicalizing NaN.
    /// Rust does not contract `f32` arithmetic into FMA without fast-math, and
    /// the sim uses no libm transcendentals, so this is stable across native
    /// builds on the same architecture.
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.u32(self.stats.frames);
        h.f32(self.stats.progress_pixels);
        h.f32(self.stats.closest_approach);
        h.f32(self.stats.best_x);
        h.u32(self.stats.no_progress_frames);
        h.u32(self.stats.stomps);
        h.u32(self.stats.jumps);
        h.u32(self.stats.checkpoints);
        h.u32(self.stats.route_hops_closed);
        h.u32(self.stats.decks);
        match self.stats.best_stand {
            Some(s) => {
                h.u8(1);
                h.u32(s as u32);
            }
            None => h.u8(0),
        }
        // And the zone ids themselves, so a change to *which* zones qualify moves this hash
        // even when an episode happens to credit the same number of them.
        h.u32(self.credited.len() as u32);
        for (row, zone) in &self.credited {
            h.u32(*row as u32);
            h.u32(*zone as u32);
        }
        // The terrain a body has changed, and the fuses still burning: the level started this
        // episode as a document and is now two different things in two runs that stood on
        // different tiles, so a hash that left this out would call those two runs the same state.
        h.u32(self.broken.len() as u32);
        for (tx, ty) in &self.broken {
            h.i32(*tx);
            h.i32(*ty);
        }
        h.u32(self.crumbles.len() as u32);
        for ((tx, ty), left) in &self.crumbles {
            h.i32(*tx);
            h.i32(*ty);
            h.u32(*left);
        }
        match self.stats.death_pos {
            Some((x, y)) => {
                h.u8(1);
                h.f32(x);
                h.f32(y);
            }
            None => h.u8(0),
        }
        h.u8(self.outcome as u8);
        self.player.hash_into(&mut h);
        h.u32(self.walkers.len() as u32);
        for w in &self.walkers {
            w.hash_into(&mut h);
        }
        // The enemies this pass added, and the shelf a body may be riding. A platform's
        // *position* is already implied by `frames`, which is the first thing hashed — what is
        // not implied is which one the body stands on, and that is state.
        h.u32(self.flyers.len() as u32);
        for f in &self.flyers {
            f.hash_into(&mut h);
        }
        h.u32(self.turrets.len() as u32);
        for t in &self.turrets {
            t.hash_into(&mut h);
        }
        h.u32(self.shots.len() as u32);
        for s in &self.shots {
            s.hash_into(&mut h);
        }
        // The pools' own attack: a ball is state, and the count is folded in even at zero so a
        // level that has gained a vent cannot hash like one that never had one.
        h.u32(self.lava_balls.len() as u32);
        for b in &self.lava_balls {
            b.hash_into(&mut h);
        }
        h.u32(self.movers.len() as u32);
        for m in &self.movers {
            m.hash_into(&mut h);
        }
        match self.riding {
            Some(i) => {
                h.u8(1);
                h.u32(i as u32);
            }
            None => h.u8(0),
        }
        h.u64(self.rng_probe());
        h.finish()
    }

    /// Deterministic probe of the RNG's internal state.
    ///
    /// `Xoshiro256PlusPlus` does not expose its state words, so hashing a clone's
    /// next output is the cheapest way to fold "where the RNG stream is" into the
    /// state hash. Two sims at the same point in the same stream always agree.
    ///
    /// Note the walkers' spawn draw already moved this stream, so two sims built
    /// from different seeds have different probes even before a single `step`.
    /// That is the intended signal: the seeds really do produce different
    /// episodes on any level with walkers.
    pub fn rng_probe(&self) -> u64 {
        use rand_xoshiro::rand_core::Rng as _;
        let mut probe = self.rng.clone();
        probe.next_u64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A time is written one way, and the two shapes meet exactly where they say they do.
    #[test]
    fn a_time_in_frames_is_written_short_and_in_order() {
        assert_eq!(format_time(0), "0.00s");
        assert_eq!(format_time(1), "0.02s", "one frame is 1/60 s");
        assert_eq!(format_time(505), "8.42s");
        assert_eq!(format_time(2400), "40.00s", "a full-width level's budget");
        // The seam: 3599 frames is 59.98 s and stays in hundredths, 3600 is a minute and changes
        // shape. A reader never sees both shapes for one time, and a time is never written two
        // ways, which is the whole point of there being one function.
        assert_eq!(format_time(3599), "59.98s");
        assert_eq!(format_time(3600), "1:00.00");
        assert_eq!(format_time(3600 + 150), "1:02.50");

        // And it is a *clock*: reading a time back gives the frame count it was made from, so a
        // column of them is ordered by frames whatever shape each one is in. Sixty frames to the
        // hundredth is exactly why hundredths are the right resolution — a frame is 0.0167 s, so
        // two episodes a frame apart still read as two times.
        for frames in [
            0u32, 1, 2, 59, 60, 505, 3598, 3599, 3600, 3725, 20_000, 727_000,
        ] {
            assert_eq!(
                frames_of(&format_time(frames)),
                frames as i64,
                "{} did not survive the round trip",
                format_time(frames)
            );
        }
    }

    #[test]
    fn a_plan_guide_counts_waypoints_once_and_never_backwards() {
        let level = Level::from_json_str(include_str!("../../../levels/l1-flat-run.json"))
            .expect("the shipped level parses");
        let mut sim = Sim::new(&level, 1);
        let start = sim.player().aabb().center();
        let goal = level.goals.first().expect("the level has a flag").center();
        // A straight line from the spawn to the flag, sampled densely enough that a body running
        // the floor can meet each point in turn.
        let points = (0..=20)
            .map(|i| {
                let t = i as f32 / 20.0;
                (start.0 + (goal.0 - start.0) * t, start.1 + (goal.1 - start.1) * t)
            })
            .collect();
        sim.set_plan_guide(PlanGuide {
            points,
            radius: 0.75,
        });
        let mut last = sim.plan_progress().0;
        for _ in 0..120 {
            sim.step(crate::physics::AgentAction::Run);
            let reached = sim.plan_progress().0;
            assert!(reached >= last, "the plan cursor must be monotone");
            last = reached;
        }
        assert!(last > 1, "a body that ran along the plan reached nothing: {last}");
        assert!(
            sim.plan_progress().0 <= sim.plan_progress().1,
            "the cursor cannot pass the guide's end"
        );
    }

    /// **The scratch observation is the observation.** `observation_into` exists so a training
    /// run stops allocating three vectors per decision; this pins that the saving changes
    /// nothing else, over an episode on a level whose movers actually move (so the mover
    /// rectangle, velocity and phase slots are live, not zeros agreeing with zeros).
    #[test]
    fn the_scratch_observation_is_bit_identical_to_the_allocating_one() {
        let level = Level::from_json_str(include_str!("../../../levels/l11-the-elements.json"))
            .expect("the shipped level parses");
        assert!(
            !Sim::new(&level, 7).movers.is_empty(),
            "the fixture level must have movers for this test to say anything"
        );
        let mut sim = Sim::new(&level, 7);
        let mut scratch = ObservationScratch::new();
        // A reused scratch across a second episode too: stale capacity must never leak
        // yesterday's movers into today's vector.
        for episode in 0..2 {
            if episode == 1 {
                sim = Sim::new(&level, 8);
            }
            for frame in 0..900 {
                if sim.outcome().is_finished() {
                    break;
                }
                let plain = sim.observation();
                let reused = sim.observation_into(&mut scratch);
                for (i, (a, b)) in plain.iter().zip(reused.iter()).enumerate() {
                    assert_eq!(
                        a.to_bits(),
                        b.to_bits(),
                        "slot {i} differs at frame {frame} of episode {episode}"
                    );
                }
                let action = if frame % 50 < 25 {
                    AgentAction::Run
                } else {
                    AgentAction::RunJump
                };
                sim.step(action);
            }
        }
    }

    /// Frames a written time stands for, to the nearest frame. The inverse of [`format_time`] for
    /// both of its shapes, used only by the test above — which is what makes "a slower episode
    /// never reads as a faster one" a fact rather than an impression.
    fn frames_of(written: &str) -> i64 {
        let (minutes, rest) = match written.split_once(':') {
            Some((m, rest)) => (m.parse::<f64>().expect("minutes"), rest),
            None => (0.0, written),
        };
        let seconds = rest.trim_end_matches('s').parse::<f64>().expect("seconds");
        let total = minutes * 60.0 + seconds;
        (total / DT as f64).round() as i64
    }
}
