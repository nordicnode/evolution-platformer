//! Observation vector builder (§7.2). **Versioned**: any change to the layout,
//! the encodings, or the action-repeat cadence is a `VERSION` bump, and every
//! champion file records it (§8).
//!
//! `N_INPUTS = 81` patch cells plus 19 scalars, 100 in all (`input_names()` is the
//! authoritative layout, `train-cli sensors` prints it). The patch is deliberately 9×9 rather
//! than 13×13: Togelius 2009 showed small controllers beating large ones, and input size
//! multiplies the search space. Widening the patch is the documented upgrade path, gated on
//! evolution visibly plateauing on levels that should be solvable; the scalars beside it have
//! grown one at a time, each with a measurement in [`VERSION`].
//!
//! v2 appends the three inputs the movement abilities need and changes what `vx`
//! means; every earlier index keeps its meaning and its position, so the diff between
//! v1 and v2 is additive and reviewable. An agent cannot reason about a wall it cannot
//! see or a sprint it cannot feel, so both are sensor changes rather than physics-only
//! ones — and that is why the version had to move: a v1 champion asked to drive a body
//! with nine actions and a wall slide would be building a policy on a lie.
//!
//! v3 replaces the horizontal `goal_dir` sign with a two-axis `goal_dx, goal_dy` vector.
//! That single sign was the observation's last assumption that the flag is always to the
//! right, and it is the one assumption a level designer can falsify in ten seconds by
//! stacking a tower: an agent at the foot of one read the same `+1` it reads at the mouth of
//! a corridor. The score had the same assumption baked in and was fixed in the same pass
//! (`EpisodeStats::progress_pixels` is now distance-to-the-flag rather than rightmost x), so
//! the sensor and the objective agree about what the goal is.
//!
//! v7 appends the nearest sighted walker's own direction of travel. A body could see *where* a
//! walker was and never *which way it was going*, and that is not a detail a level can state:
//! a patrol's initial direction is drawn from the episode RNG
//! ([`Walker::spawn_spread`](crate::physics::Walker::spawn_spread)), which is v1's only gameplay
//! randomness and the reason a trial count means anything at all on a walker level. So the one
//! fact about a patrol that decides whether to cross in front of it was, until this slot,
//! unobservable rather than merely unlearned.
//!
//! v4 splits the single `gap_ahead` probe into `gap_right` and `gap_left`. The one probe was
//! the observation's last directional assumption, and it was the subtlest of the three: it
//! asked about the side the body was *moving* toward — `PlayerState::facing`, which reads
//! "right" whenever the body is standing still — so an agent at rest at the lip of a pit on
//! its left was told about the ground under its nose and nothing about the hole it was about
//! to choose. Every other sensor here describes the body or the world; that one described the
//! body's *last decision*, which is the one thing a sensor must not do, because the first
//! decision of an episode is exactly when a direction has to be chosen. Bumped rather than
//! patched in place: index 83 keeps its slot but changes meaning (`gap_ahead` as read by a
//! rightward body) and index 94 is new, which re-scores every champion ever trained.

use crate::level::{Level, Rect, Tile};
use crate::physics::{AgentAction, Flyer, LavaBall, Physics, PlayerState, Shot, Walker};

/// Sensor layout version. Bump on *any* change below (§8).
///
/// v2: the appended wall and sprint inputs, and `vx` renormalised from `run_speed` to
/// `sprint_speed` (see [`observe`]).
///
/// v3: the single horizontal `goal_dir` sign became the two-component `goal_dx, goal_dy`
/// vector below. The sign was the last place in the *observation* that assumed the goal is
/// to the right, and it made a level's goal invisible whenever the answer was "up": an
/// agent standing at the foot of a tower read `+1` for a flag directly above its head, which
/// is the same reading as a flag at the end of a corridor.
///
/// v4: `gap_ahead` — a probe whose direction was `sign(vx)`, and therefore "right" for a
/// body at rest — became `gap_right` and `gap_left`, one slot per direction. See the module
/// docs.
/// v5: the nearest-enemy sensor is bounded to [`ENEMY_VIEW_DISTANCE`] so a walker far across
/// the map (or outside it) reads `(0.0, 0.0)` instead of saturating the threat vector from
/// tick 0.
///
/// v7: `enemy_dir`, the nearest sighted walker's direction of travel, is appended at index 95 —
/// the last index, so every earlier slot keeps its meaning and its position. See the module
/// docs for why the direction is not something the level format can state.
///
/// v7 is an *addition* rather than a re-reading, so there is one invariance to check and no
/// sweep to report: the levels that cannot see it did not get harder. `l3-gauntlet`, twelve
/// seeds at pop 200, `--stop-on-win`: **12 / 12 solved at a mean first-win generation of 17.9**
/// against the 12 / 12 at 18 that §8 of `REVIEW.md` records for v6 — the same reading, on a
/// genome that is sixteen genes larger. (No arm-level before/after exists for the slot itself:
/// v6 is not a build this tree still has, and the honest claim is that the information went up
/// and nothing regressed, not that a number moved.)
///
/// v6: that bound moved from four tiles to eight. Four was chosen to match
/// [`PATCH_RADIUS`] — "the scalar says no more than the grid does" — and it made the walker
/// levels *harder to learn* rather than easier, because a threat that is only legible once it
/// is already inside the patch is a threat the population meets at contact range. Twelve
/// seeds at pop 200 on `l3-gauntlet`, each stopping at its first win:
///
/// | enemy view | solved | mean first-win generation | wall clock, 12 runs |
/// |---|---|---|---|
/// | v4, unbounded | 12 / 12 | 25 | 3.8 s |
/// | v5, four tiles | 12 / 12 | **41** | 15.2 s |
/// | v6, eight tiles | 12 / 12 | **18** | 6.9 s |
///
/// So the phantom v5 removed was worth removing (it is a lie about where the threat is), but
/// four tiles over-corrected: it took a walker out of the observation until it was inside the
/// patch, and the population then learned to reach the flag later than it did with the
/// unbounded, dishonest vector. Eight is the range at which a walker is a thing to plan
/// around rather than a thing to collide with, it still reads `(0.0, 0.0)` for the walker on
/// the far side of a 40-tile level — which is the phantom v5 set out to remove — and it is
/// the only one of the three that also *beats* the committed v4 vector on the level that
/// exposed the problem. The levels without walkers cannot see this change at all: between the
/// four-tile and the eight-tile arm, `scratch-tower` is 10 of 12 at a mean first-win
/// generation of 94 in both and `l2-first-gap`, `l5-tightrope` and `l7-ascent` are 12 of 12 at
/// generations 2, 16 and 10 in both. (Those three levels do read the *scoring* change of the
/// same pass — directional checkpoint credit — and the tower is measurably quicker for it, 94
/// against the committed build's 122 at the same 10 of 12; that is a different arm and it is
/// recorded as one in `REVIEW.md`.) The grid is unchanged: this is the scalar's own range.
///
/// v9: the two tiles the trap pass added — a cracked tile and a spring pad — are encoded in the
/// patch rather than folded into [`ENC_SOLID`] and [`ENC_EMPTY`]. Nothing about the *layout*
/// moved: not one index changed meaning and [`N_INPUTS`] is unmoved, which is the cheapest a
/// sensor change can be. What moved is what a cell of the grid **says**, and it had to, because
/// both tiles are exactly the case the sand precedent does not cover.
///
/// Sand is a material: it is solid, and physics draws no distinction between it and soil, so a
/// sensor that did would be inventing a problem. A cracked tile is not a material — it is
/// ordinary ground a body is about to lose, and a spring is a cell that will throw a body many
/// tiles into the air. A patch that reported either as plain solid or plain air would be telling
/// the population something false about the world, and both lies are the expensive kind: the
/// first hides the only cue that a bridge is a bridge, and the second makes the single most
/// useful object in a level invisible at the moment a body is deciding whether to jump onto it.
///
/// It is a **re-reading** rather than an addition, so it re-scores every champion (it is a
/// version bump, and the champion loader is what enforces that) without changing a single one's
/// shape: a champion trained on the levels that have neither tile observes byte-identical inputs
/// and plays exactly as it did.
///
/// v10 appends `phase_x, phase_y` at indices 98 and 99: **where the body is inside the tile the
/// patch is keyed on**. It is here because of a measurement, and the measurement has two halves.
///
/// The observation without it is *tile-keyed*: the patch sits on `floor(centre)`, the two gap probes
/// on the body's own edges, and the only continuous position signal anywhere in the vector is the
/// goal vector — which is **clamped** at [`GOAL_VIEW_DISTANCE`]. So for a body approaching an
/// obstacle further from the flag than that horizon, a state and the same state a fifth of a tile
/// later are one vector, and the press has nothing to be timed by. (Stated precisely, because it was
/// checked: no two decisions in any demonstrated walk over the shipped pack share a vector and
/// disagree about the action — the pack's obstacles are mostly inside the horizon, where the goal
/// delta is a fine but *nonlinear* position signal. This slot is the same information as a straight
/// line the fit can threshold, and that is the difference the second half measures.)
///
/// And the second half, which is what actually got it built: `evo::imitate` fits a network to the
/// walk `evo::route_play` makes, and its visible failure is *the press* — a clone walks to the pit and
/// steps in. With these two slots a 4000-epoch fit misses **0 to 2** of a demonstration's decisions
/// where the 98-wide vector misses **18 to 19** of them; and at the oracle's own tick cadence a fit
/// that misses nothing solves `l2-first-gap`, `l4-rhythm` and `l5-tightrope` on all eight seeds where
/// the one that misses eighteen solves none of them. It does not make a clone *play* at the shipped
/// four-tick cadence — that is `evo::imitate`'s own finding, and it is about the cadence rather than
/// about the vector — but it is what makes the demonstrated press a function a fit can reach.
///
/// **v11 makes three slots mean more without making the vector wider.** The nearest-threat
/// triple (indices 81, 82 and 95) now answers about flyers and shots as well as walkers — the
/// nearest of the three wins, ties keeping the earlier kind — and the patch gained
/// [`ENC_ONEWAY`] for the one-way tile and for a moving platform's own cells. Both are
/// *readings* of existing slots rather than new inputs, which is the version number's job: a
/// pack level with none of the new kinds observes the identical vector it did at v10, and a
/// champion trained on one is refused rather than quietly played against a changed reading.
/// **v12 gives the patch the elements**, in three values of its own and no new slots: water is
/// air that pulls a body down, a vine is air a body can hold, and lava is a cell that kills.
///
/// All three are *readings* rather than inputs, which is the cheap kind of sensor change — the
/// vector is still [`N_INPUTS`] wide and not one index moved — but none of them can be folded
/// into [`ENC_SOLID`] or [`ENC_EMPTY`] without lying about the world in the expensive way v9
/// describes. Air with a vine in it is a route; air over water is a slow sink a body must swim
/// out of; and a cell that ends the episode must not look like a place to stand.
///
/// The one deliberate economy is that **lava is encoded exactly as a spike is** ([`ENC_LAVA`] is
/// [`ENC_SPIKE`]): the two are the same decision — do not go in — and giving them different
/// numbers would ask a network to learn a distinction the action set cannot express. Water and
/// vine *are* distinct, because their decisions are: at water the press is the answer, at a vine
/// the press is the climb.
///
/// v11 read the twin triple and the patch's one-way value for the new kinds; v12 is the same
/// shape of change for the tiles themselves.
///
/// **v14 appends the rope** at indices 108 to 110: whether the body is holding one, where the
/// nearest line hangs (`rope_dx` to the line, `rope_dy` to its end) and nothing else. It is a
/// new *slot* rather than a new reading of an old one, because a rope is not a tile: the patch
/// has no cell to put it in, and the one thing a policy has to know about a rope is *where the
/// line is relative to the hands*, which is what the pair of offsets says.
///
/// The three slots are appended, so a champion grown from the previous layout keeps every
/// weight where it was and reads zeros where the rope would be — see `Genome::grown_to_inputs`.
/// That is the whole reason growth is append-only: the levels that have no rope observe exactly
/// the same vector they observed at v13, and a champion trained on one still plays it.
///
/// **v15 puts the spiked ball in the patch**, as [`ENC_SPIKE`]. No slot moved and
/// [`N_INPUTS`] is unchanged. It is a bump rather than a quiet reading because a cell that ends
/// the episode is no longer only terrain: a level with a ball now says "do not go in" about a
/// picture that was not there at v14. The value itself is the one the spike already used — the
/// decision at a ball and at a spike is the same one — which is the same economy v12 made by
/// giving lava the spike's value. A level without a ball observes exactly the vector it observed
/// at v14.
pub const VERSION: u32 = 15;

/// Tiles per side of the local patch (`9×9 = 81` inputs).
pub const PATCH_RADIUS: i32 = 4;
/// Number of inputs in the observation vector.
///
/// **The enemy slots are a triple, and the third one is the direction**: `enemy_dx, enemy_dy` say
/// where the nearest walker in sight is and `enemy_dir` says which way it is walking, so a body
/// at the mouth of a corridor can answer "can I cross now" from one frame. The third was the only
/// one of the three that no amount of training could recover, because the level format does not
/// hold it — a patrol's initial direction is drawn from the episode RNG.
///
/// **v8 appends the route's next waypoint (`waypoint_dx, waypoint_dy`)** at indices 96 and 97.
/// Straight-line `goal_dx, goal_dy` points through ceilings on zig-zag towers (`scratch-spike-tower`),
/// telling a memoryless policy at the foot of a left-going leg to run right. The waypoint vector
/// points from the body to the next landing stand along [`crate::reach::GoalRoutes`] (or to the
/// flag itself once on the final stand), normalised by [`GOAL_VIEW_DISTANCE`].
///
/// **v10 appends the body's sub-tile phase (`phase_x, phase_y`)** at indices 98 and 99: the part of
/// the body's position inside its own tile, which is the one positional quantity no other slot
/// carries once the goal vector is clamped. See [`VERSION`] for the measurement.
pub const N_INPUTS: usize = 81 + 2 + 1 + 1 + 2 + 1 + 2 + 1 + 3 + 1 + 1 + 2 + 2 + 8 + 3;

/// Inputs appended in v2: wall side, wall slide, sprint charge.
pub const N_ABILITY_INPUTS: usize = 3;

/// Normalizer for the nearest-enemy vector (tiles), and the range beyond which a walker is
/// simply not in the observation.
///
/// Eight, not [`PATCH_RADIUS`]: the vector's job is the warning the patch cannot give, and the
/// measurement in [`VERSION`]'s v6 note is why. A walker further away on either axis is out of
/// sight in both channels, so a level's far end is quiet rather than a constant threat.
pub const ENEMY_VIEW_DISTANCE: f32 = 8.0;
/// How far each gap probe looks, in tiles.
///
/// Three, the radius of the patch, so the probe is "the patch, summarised": a hole at the
/// fourth tile is visible in the 9×9 grid but not worth a dedicated scalar, which is the
/// division of labour between the patch and the feature slots.
pub const GAP_LOOKAHEAD: i32 = 3;

/// Normalizer for the nearest rope's offsets (tiles).
///
/// Eight, like [`ENEMY_VIEW_DISTANCE`] and for the same kind of reason: a rope is only useful
/// within a jump of the body, and an offset that saturated over the whole level would tell a
/// policy "a rope exists somewhere" rather than "a rope is a jump to my right".
pub const ROPE_VIEW_DISTANCE: f32 = 8.0;

/// Normalizer for the goal vector (tiles).
///
/// Much longer than [`ENEMY_VIEW_DISTANCE`], and for a different job: the enemy vector is a
/// *threat* that only matters up close, while the goal is the one thing in the level that is
/// always relevant wherever it is. Sixteen tiles keeps useful resolution over most of a
/// shipped level (60-80 tiles wide, so the far half of a long level saturates) while still
/// reading "directly above me, about a jump away" precisely, which is where the precision
/// matters.
pub const GOAL_VIEW_DISTANCE: f32 = 16.0;

/// Tile encodings inside the patch.
pub const ENC_SOLID: f32 = 1.0;
pub const ENC_SPIKE: f32 = -0.5;
pub const ENC_EMPTY: f32 = 0.0;
pub const ENC_ENEMY: f32 = -1.0;

/// A spring pad: a cell that is not ground and that a body *wants* under it.
///
/// Positive, and above [`ENC_EMPTY`] rather than beside [`ENC_SOLID`]: the value's job is to read
/// as "worth aiming at", which is the one thing a plain solid tile does not say, and the network
/// has no reason to learn a mapping that puts it on the bright side of air.
pub const ENC_SPRING: f32 = 0.5;

/// A one-way platform, or a moving one: **ground that lets a body through from below.**
///
/// A value of its own rather than [`ENC_SOLID`], because the two states ask for different
/// actions: a solid cell four above the body is a ceiling to stop jumping into, and a shelf in
/// the same place is a route to jump through and land on. Encoding it as solid would teach a
/// policy to refuse exactly the move the tile exists to reward, and encoding it as air would
/// hide the landing. It sits between air and ground, which is what it is.
pub const ENC_ONEWAY: f32 = 0.75;

/// A cracked tile: ground, and ground that is about to stop being ground.
///
/// Negative and *between* air and a spike — the ordering is the design. A cracked tile is a
/// smaller danger than a spike (it drops the body, it does not kill it) and a larger one than
/// open air, which costs the body nothing until it lands somewhere; a body that reads the three
/// as ordered learns one axis rather than three unrelated labels. Like every value in the patch
/// it says what the cell *is*, never what to do about it.
pub const ENC_CRUMBLE: f32 = -0.25;

/// A water cell: **air that pulls a body down**.
///
/// Just below [`ENC_EMPTY`] and just above a cracked tile, which is the ordering of the trouble
/// they are: a body can cross water at any speed it can swim, so it is not an obstacle, but it is
/// the only cell in the game a body has to *press a button* to get out of. A value of its own
/// rather than `ENC_EMPTY` because a policy that reads water as air learns to fall into pools and
/// never to swim — the water's whole lesson is "hold jump".
pub const ENC_WATER: f32 = -0.1;

/// A lava cell: **death**, encoded exactly as a spike is.
///
/// The same number on purpose. A spike and a lava cell are the same decision (do not be here),
/// the action set has no way to treat them differently, and two adjacent values would ask a fit
/// to separate two things whose correct answer is identical.
pub const ENC_LAVA: f32 = ENC_SPIKE;

/// A vine: **air a body can hold on to**.
///
/// Positive but below a spring (0.5) and a shelf (0.75): the ordering is "how much good this does
/// a body that is in it", and a vine is good only while the button is down. A climber's cue is
/// the cell itself; the value says "this is not a hole", which is the one thing air would say
/// wrong.
pub const ENC_VINE: f32 = 0.25;

/// The plan's sensor table omits tiles for spikes, which would leave the single
/// most common hazard invisible to the agent. Spikes therefore get
/// [`ENC_SPIKE`] — distinct from, and less alarming than, an enemy.
#[inline]
fn encode_tile(t: Tile) -> f32 {
    match t {
        // Sand, brick and plank are solid, and an agent is told so: the observation is about
        // what a body can do, not what the world is made of. A material that looked different
        // to the sensors would be a new problem for the population to solve, which is the one
        // thing adding a *colour* of ground must never be.
        // A belt is ground, and it reads as ground. Its *direction* is not in this table, and
        // that is a decision: the patch is the world a body can act on, and a belt is a floor to
        // stand on like any other. Being carried shows up where the body feels it — in `vx` — so
        // a policy learns a belt by standing on one rather than by reading a code for it, and
        // adding a code would have meant a sensor version that invalidates every champion on
        // disk for a cell whose action is "walk on it".
        Tile::Solid | Tile::Sand | Tile::Brick | Tile::Plank => ENC_SOLID,
        Tile::BeltRight | Tile::BeltLeft => ENC_SOLID,
        // A cracked tile is solid for the same reason and until the same moment, and then it is
        // not: the variant is a fact about the ground's *future* rather than about what it is
        // made of, which is why it reads differently here and not differently anywhere else.
        Tile::Cracked | Tile::CrackedSand | Tile::CrackedBrick | Tile::CrackedPlank => ENC_CRUMBLE,
        Tile::Spike => ENC_SPIKE,
        Tile::Spring => ENC_SPRING,
        // Ground that is also a doorway: see [`ENC_ONEWAY`].
        Tile::OneWay => ENC_ONEWAY,
        Tile::Water => ENC_WATER,
        Tile::Lava => ENC_LAVA,
        Tile::Vine => ENC_VINE,
        Tile::Empty | Tile::Goal => ENC_EMPTY,
    }
}

/// Build the observation vector for the current sim state.
///
/// Layout (all values finite; unused slots are `0.0`):
///
/// | offset | count | meaning |
/// |---|---|---|
/// | 0   | 81 | 9×9 tile patch, row-major, top-left first |
/// | 81  | 2  | nearest live enemy `dx, dy`, normalised by [`ENEMY_VIEW_DISTANCE`] |
/// | 83  | 1  | gap to the right of the feet (1.0 = no ground within [`GAP_LOOKAHEAD`] tiles) |
/// | 84  | 1  | grounded |
/// | 85  | 2  | `vx`, `vy` normalised by `sprint_speed` / `max_fall` |
/// | 87  | 1  | can jump (grounded or coyote) |
/// | 88  | 2  | goal `dx, dy` from the body, normalised by [`GOAL_VIEW_DISTANCE`] |
/// | 90  | 1  | bias, constant 1.0 |
/// | 91  | 1  | wall side pressed into (−1 left / +1 right / 0 none) |
/// | 92  | 1  | wall slide active |
/// | 93  | 1  | sprint charge, 0 at `run_speed` and 1 at `sprint_speed` |
/// | 94  | 1  | gap to the left of the feet |
/// | 95  | 1  | the same walker's direction of travel (+1 right, −1 left, 0 none in sight) |
/// | 96  | 2  | route `dx, dy` to the next landing stand, normalised by [`GOAL_VIEW_DISTANCE`] |
/// | 98  | 2  | where the body is inside its own tile, each axis in −1..1 |
///
/// The v7 slot describes *the walker* the two slots above it describe, not a second, nearer one:
/// a level with two walkers in sight reports the closest pair's position and that walker's
/// heading, which is one fact about one body rather than three facts about two. A body that wants
/// to reason about the other one has the patch, and the tie-break is walker index, so the reading
/// is stable for a given frame.
///
/// The two gap slots are a **pair, and neither is "forward"**. What they describe is the
/// ground immediately beside each edge of the body, which is the thing a jump decision is
/// about, and which side that is has nothing to do with where the body is already drifting.
///
/// Two notes on the v2 inputs. `vx` is normalised by `sprint_speed` rather than
/// `run_speed` on purpose: it used to saturate at 1.0 for a running agent, which would
/// have made a sprinting body and a running one *indistinguishable* — the one thing a
/// speed sensor exists to tell apart. Running now reads 0.67 and sprinting 1.0.
///
/// The wall side is the wall the body is *pressed against*, not merely next to. That is
/// deliberate: an agent cannot see "a wall is one tile to my right" from a sensor that
/// only fires on contact, but it can see the wall in its own 9×9 patch, so what it needs
/// from this input is the state of its own cling — and the patch plus this input is
/// enough to learn the press-then-jump rhythm a wall climb is made of.
pub fn observe(
    level: &Level,
    player: &PlayerState,
    walkers: &[Walker],
    ph: &Physics,
) -> [f32; N_INPUTS] {
    let stand = if player.grounded {
        let surface = Level::tile_y(player.y + player.h + 0.05);
        let col = Level::tile_x(player.x + player.w * 0.5);
        level.routes().zone_at(col, surface)
    } else {
        None
    };
    observe_from_stand(level, player, walkers, ph, stand)
}

/// Build the observation vector when the body's current or last grounded route stand is known.
pub fn observe_from_stand(
    level: &Level,
    player: &PlayerState,
    walkers: &[Walker],
    ph: &Physics,
    current_stand: Option<usize>,
) -> [f32; N_INPUTS] {
    observe_scene(
        level,
        player,
        &Scene {
            walkers,
            flyers: &[],
            shots: &[],
            lava_balls: &[],
            movers: &[],
            mover_vel: &[],
            mover_phase: &[],
            turrets: &[],
            spike_balls: &[],
            blocks: &[],
            rocks: &[],
            vents: &[],
            air_ticks: 0,
            ground_ticks: 0,
            frames: 0,
        },
        ph,
        current_stand,
    )
}

/// Everything in a frame that is not terrain, as the observation sees it.
///
/// A bundle rather than five arguments: the three threat kinds share one vector (the nearest
/// of *any* of them is what the enemy slots describe), and the movers share the patch's
/// encoding because a platform is ground that happens to move. Keeping this a struct is what
/// lets [`observe_from_stand`] remain the walker-only entry point every existing caller and
/// test already uses.
pub struct Scene<'a> {
    pub walkers: &'a [Walker],
    pub flyers: &'a [Flyer],
    pub shots: &'a [Shot],
    /// Lava balls in flight: the pools' vertical threats, and the reason the triple reads a
    /// fourth kind. A ball is in the air a body has to jump through, exactly as a flyer is.
    pub lava_balls: &'a [LavaBall],
    /// Moving platforms, as the boxes they occupy this frame.
    pub movers: &'a [Rect],
    /// The same movers' velocity this frame, aligned with [`Scene::movers`].
    pub mover_vel: &'a [(f32, f32)],
    /// Where each mover is inside its cycle, in `0..1`, aligned with [`Scene::movers`].
    pub mover_phase: &'a [f32],
    /// The level's turrets, so the observation can say *when* the next shot leaves.
    pub turrets: &'a [crate::physics::Turret],
    /// Spiked balls on chains, at this frame's positions. A ball is drawn into the patch as a
    /// [`ENC_SPIKE`] cell rather than given slots of its own — see [`VERSION`]'s v15 note.
    pub spike_balls: &'a [crate::physics::SpikeBall],
    /// Crates, at this frame's positions, drawn into the patch as [`ENC_SOLID`] cells.
    ///
    /// A crate is solid, so it reads as solid: the policy sees a wall where the crate is and an
    /// empty cell where it was, which is exactly the feedback pushing needs. No slot moved and
    /// [`VERSION`] did not bump, for the belt's reason and the spiked ball's economy: the value is
    /// one the patch already had, and a level with no crate observes the vector it always did.
    pub blocks: &'a [crate::physics::Block],
    /// Rocks, at this frame's positions, drawn into the patch as [`ENC_SOLID`] cells.
    ///
    /// A rock is a solid whether it is hanging, falling or settled, so it reads as the block it
    /// is. It matters most *after* it has fallen: a settled rock is a one-tile step in a corridor,
    /// and a policy that could not see it would walk into an obstacle the level's own terrain says
    /// is not there. No slot moved and [`VERSION`] did not bump, for the crate's reason: the value
    /// is one the patch already had, and a level with no rock observes the vector it always did.
    pub rocks: &'a [crate::physics::Rock],
    /// The level's lava vents, so the observation can say when the next ball is thrown.
    pub vents: &'a [(i32, i32)],
    /// Ticks the body has been off the ground, and on it. Press timing is a function of both.
    pub air_ticks: u32,
    pub ground_ticks: u32,
    /// The episode's own clock, in physics ticks.
    pub frames: u32,
}

/// Build the observation vector from a whole frame.
pub fn observe_scene(
    level: &Level,
    player: &PlayerState,
    scene: &Scene<'_>,
    ph: &Physics,
    current_stand: Option<usize>,
) -> [f32; N_INPUTS] {
    let walkers = scene.walkers;
    let mut out = [0.0f32; N_INPUTS];
    let (cx, cy) = player.aabb().center();
    let ctx = cx.floor() as i32;
    let cty = cy.floor() as i32;

    // ---- 9×9 tile patch -------------------------------------------------
    let mut i = 0;
    for dy in -PATCH_RADIUS..=PATCH_RADIUS {
        for dx in -PATCH_RADIUS..=PATCH_RADIUS {
            out[i] = encode_tile(level.get(ctx + dx, cty + dy));
            i += 1;
        }
    }
    debug_assert_eq!(i, 81);

    // A moving platform is drawn into the patch as what it is — ground from above — rather
    // than as a hole, so an agent can plan a landing on a shelf it can see coming.
    for m in scene.movers {
        let (c0, c1) = (
            m.x.floor() as i32 - ctx,
            (m.x + m.w - 0.001).floor() as i32 - ctx,
        );
        let r = m.y.floor() as i32 - cty;
        if !(-PATCH_RADIUS..=PATCH_RADIUS).contains(&r) {
            continue;
        }
        for c in c0.max(-PATCH_RADIUS)..=c1.min(PATCH_RADIUS) {
            let idx = ((r + PATCH_RADIUS) * (2 * PATCH_RADIUS + 1) + (c + PATCH_RADIUS)) as usize;
            if out[idx] == ENC_EMPTY {
                out[idx] = ENC_ONEWAY;
            }
        }
    }

    // A spiked ball is a cell that ends an episode, so it is drawn into the patch as one. It is
    // the patch's existing "do not go in" value and not a new encoding: the decision at a ball
    // and at a spike is the same one, and giving it a number of its own would ask the network to
    // learn a distinction the action set cannot express. The box is floored like a mover's, so a
    // ball between two cells shows up in both — the hazard is bigger than the grid under it.
    for b in scene.spike_balls {
        let ball = b.aabb();
        let (c0, c1) = (
            ball.x.floor() as i32 - ctx,
            (ball.x + ball.w - 0.001).floor() as i32 - ctx,
        );
        let (r0, r1) = (
            ball.y.floor() as i32 - cty,
            (ball.y + ball.h - 0.001).floor() as i32 - cty,
        );
        for r in r0.max(-PATCH_RADIUS)..=r1.min(PATCH_RADIUS) {
            for c in c0.max(-PATCH_RADIUS)..=c1.min(PATCH_RADIUS) {
                let idx =
                    ((r + PATCH_RADIUS) * (2 * PATCH_RADIUS + 1) + (c + PATCH_RADIUS)) as usize;
                if out[idx] == ENC_EMPTY {
                    out[idx] = ENC_SPIKE;
                }
            }
        }
    }

    // A crate is ground that moves when a body leans on it, and it is drawn as the ground it is.
    // Solid rather than one-way: a body cannot walk through a crate, so a patch that let a policy
    // plan straight through one would be describing a level nobody is playing. The box is floored
    // like a ball's, so a crate between two cells shows up in both.
    for b in scene.blocks {
        let block = b.aabb();
        let (c0, c1) = (
            block.x.floor() as i32 - ctx,
            (block.x + block.w - 0.001).floor() as i32 - ctx,
        );
        let (r0, r1) = (
            block.y.floor() as i32 - cty,
            (block.y + block.h - 0.001).floor() as i32 - cty,
        );
        for r in r0.max(-PATCH_RADIUS)..=r1.min(PATCH_RADIUS) {
            for c in c0.max(-PATCH_RADIUS)..=c1.min(PATCH_RADIUS) {
                let idx =
                    ((r + PATCH_RADIUS) * (2 * PATCH_RADIUS + 1) + (c + PATCH_RADIUS)) as usize;
                if out[idx] == ENC_EMPTY {
                    out[idx] = ENC_SOLID;
                }
            }
        }
    }

    // A rock is a box of stone wherever it is: hanging over a corridor, on its way down, or
    // settled into the floor. Solid, like the crate, and floored like a ball's box, so a rock
    // between two cells shows up in both.
    for rock in scene.rocks {
        let box_ = rock.aabb();
        let (c0, c1) = (
            box_.x.floor() as i32 - ctx,
            (box_.x + box_.w - 0.001).floor() as i32 - ctx,
        );
        let (r0, r1) = (
            box_.y.floor() as i32 - cty,
            (box_.y + box_.h - 0.001).floor() as i32 - cty,
        );
        for r in r0.max(-PATCH_RADIUS)..=r1.min(PATCH_RADIUS) {
            for c in c0.max(-PATCH_RADIUS)..=c1.min(PATCH_RADIUS) {
                let idx =
                    ((r + PATCH_RADIUS) * (2 * PATCH_RADIUS + 1) + (c + PATCH_RADIUS)) as usize;
                if out[idx] == ENC_EMPTY {
                    out[idx] = ENC_SOLID;
                }
            }
        }
    }

    // ---- nearest live enemy (within ENEMY_VIEW_DISTANCE) ---------------
    // Ties break on walker index, so the observation is order-stable. Walkers beyond
    // ENEMY_VIEW_DISTANCE on both axes are out of sight and leave the slots at (0, 0),
    // preventing a distant walker from feeding a constant phantom threat from spawn.
    let mut best: Option<(f32, f32, i8)> = None;
    let mut best_d2 = f32::INFINITY;
    // One vector for every threat, nearest wins. Ties keep the earlier kind, which is why the
    // order is walkers, then flyers, then shots: the three the pack has had longest come first,
    // so a level with none of the new kinds observes exactly what it observed before.
    let consider =
        |bx: f32, by: f32, dir: i8, best: &mut Option<(f32, f32, i8)>, best_d2: &mut f32| {
            let (dx, dy) = (bx - cx, by - cy);
            if dx.abs() > ENEMY_VIEW_DISTANCE || dy.abs() > ENEMY_VIEW_DISTANCE {
                return;
            }
            let d2 = dx * dx + dy * dy;
            if d2 < *best_d2 {
                *best_d2 = d2;
                *best = Some((dx, dy, dir));
            }
        };
    for w in walkers.iter().filter(|w| w.alive) {
        let (wx, wy) = w.aabb(ph).center();
        consider(wx, wy, w.dir, &mut best, &mut best_d2);
    }
    for f in scene.flyers.iter().filter(|f| f.alive) {
        let (fx, fy) = f.aabb().center();
        consider(fx, fy, f.dir, &mut best, &mut best_d2);
    }
    for s in scene.shots.iter().filter(|s| s.alive) {
        let (sx, sy) = s.aabb().center();
        consider(
            sx,
            sy,
            if s.vx < 0.0 { -1 } else { 1 },
            &mut best,
            &mut best_d2,
        );
    }
    // A lava ball, last for the same reason: the three kinds the pack has had longest keep
    // their tie-break. Its heading is its lateral drift, which is the only horizontal motion
    // it has — the arc itself is vertical and the two slots above already say where it is.
    for b in scene.lava_balls.iter().filter(|b| b.alive) {
        let (bx, by) = b.aabb().center();
        consider(
            bx,
            by,
            if b.vx < 0.0 { -1 } else { 1 },
            &mut best,
            &mut best_d2,
        );
    }
    if let Some((dx, dy, dir)) = best {
        out[81] = (dx / ENEMY_VIEW_DISTANCE).clamp(-1.0, 1.0);
        out[82] = (dy / ENEMY_VIEW_DISTANCE).clamp(-1.0, 1.0);
        // v7: which way *it* is going, signed like `wall_dir` below and taken from the body's
        // own state rather than from its last position, so it is exact on the tick a walker
        // turns around as well as on the ticks in between.
        out[95] = dir as f32;
    }

    // ---- ground beside each edge of the feet ---------------------------
    // Measured from the edge the body would walk off (its right edge going right, its left
    // edge going left), so each probe answers "if I commit to this side, do I leave the
    // ground here?". The row is the one just under the feet, which is the ground itself.
    let foot_row = (player.y + player.h + 0.05).floor() as i32;
    out[83] = gap_beside(level, (player.x + player.w).floor() as i32, 1, foot_row);
    out[94] = gap_beside(level, player.x.floor() as i32, -1, foot_row);

    // ---- grounded, velocity, can-jump, goal direction, bias -------------
    out[84] = if player.grounded { 1.0 } else { 0.0 };
    out[85] = (player.vx / ph.sprint_speed).clamp(-1.0, 1.0);
    out[86] = (player.vy / ph.max_fall).clamp(-1.0, 1.0);
    out[87] = if player.can_jump() { 1.0 } else { 0.0 };

    // ---- v3: where the flag actually is --------------------------------
    let (gdx, gdy) = goal_delta(level, player);
    out[88] = (gdx / GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0);
    out[89] = (gdy / GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0);
    out[90] = 1.0;

    // ---- v2: the movement abilities -------------------------------------
    out[91] = player.wall_dir() as f32;
    out[92] = if player.clinging() { 1.0 } else { 0.0 };
    let span = ph.sprint_speed - ph.run_speed;
    out[93] = if span > 0.0 {
        ((player.speed_cap() - ph.run_speed) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // ---- v8: next route waypoint ----------------------------------------
    let (wdx, wdy) = waypoint_delta(level, player, current_stand, (gdx, gdy));
    out[96] = (wdx / GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0);
    out[97] = (wdy / GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0);

    // ---- v10: where the body is inside its own tile ----------------------
    // The patch above is keyed on `(cx.floor(), cy.floor())`, so every other input is a function of
    // the body's *tile*. These two slots are the remainder — `0` at the tile's own origin, rising
    // toward the next boundary — and they are what a press has to be timed by. Mapped to `[-1, 1)`
    // so the slot reads like the velocities beside it wherever the body is in its tile, and
    // centre-based so it is the exact complement of the patch's key rather than a second, slightly
    // different notion of position.
    out[98] = (2.0 * (cx - ctx as f32) - 1.0).clamp(-1.0, 1.0);
    out[99] = (2.0 * (cy - cty as f32) - 1.0).clamp(-1.0, 1.0);

    // ---- v13: time ----------------------------------------------------
    //
    // The five mechanics added since v11 are all *timed*, and every sensor before this block
    // described where a thing was rather than when it would act. That is a fact no amount of
    // training can recover: a policy that cannot see the turret's cooldown cannot learn "wait
    // for the gap", and a policy that cannot see the body's own clock cannot commit to a plan.
    // These eight slots are the smallest bundle that makes the pack's timed crossings
    // *expressible*: the body's clock, the next dangerous event (how long, and where), and the
    // shelf under the body (how fast, and where in its cycle).
    // The clock is **coarse on purpose**. A per-frame counter makes every state in an episode
    // unique, so a memoryless fit can index the episode instead of the world: measured on
    // `l2-first-gap` at the one-tick cadence, a fit that reproduced every old slot still lost the
    // pit once the raw frame count was in the vector. Sixteen steps over 64 frames each keep a
    // rough "how long have I been waiting" while the event eta carries the precision.
    out[100] = (((scene.frames / 64) % 16) as f32 / 16.0).clamp(0.0, 1.0);
    if let Some((eta, ex, ey)) = next_event(level, player, scene, ph) {
        out[101] = (eta / 120.0).clamp(0.0, 1.0);
        out[102] = ((ex - cx) / ENEMY_VIEW_DISTANCE).clamp(-1.0, 1.0);
        out[103] = ((ey - cy) / ENEMY_VIEW_DISTANCE).clamp(-1.0, 1.0);
    }

    // The nearest shelf, as velocity and cycle phase. Chosen by distance so a body on a deck
    // reads the deck it is standing on; the patch already says where it is.
    if let Some((vx, vy, phase)) = nearest_mover(player, scene) {
        out[104] = (vx / ph.mover_speed.max(1e-6)).clamp(-1.0, 1.0);
        out[105] = (vy / ph.mover_speed.max(1e-6)).clamp(-1.0, 1.0);
        out[106] = phase.clamp(0.0, 1.0);
    }
    // Which half of a press's timing the body is in: fresh on the ground, or falling, or rising.
    // The signed form is deliberate — a policy that needs "release now" and one that needs
    // "hold through the apex" can both read it, and it is one slot rather than two.
    out[107] = if player.grounded {
        -(scene.ground_ticks as f32 / 120.0).clamp(0.0, 1.0)
    } else {
        (scene.air_ticks as f32 / 120.0).clamp(0.0, 1.0)
    };

    // ---- the rope -------------------------------------------------------
    //
    // `on_rope` is a state, and the pair after it is the **aim**: the horizontal offset to the
    // nearest line and the vertical offset to that line's end. Both are relative to the body's
    // centre, so a policy can answer "is the line in front of me and is its end still below my
    // hands" without a second sense — which is the question a grab is made of. The nearest line
    // is the one whose segment passes nearest the body, measured to the line itself rather than
    // to its anchor, because the anchor may be ten tiles up and out of sight.
    out[108] = if player.on_rope() { 1.0 } else { 0.0 };
    let mut nearest_rope: Option<(f32, f32, f32)> = None;
    for rope in &level.ropes {
        let (ax, ay) = rope.anchor();
        let end = ay + rope.length;
        let (bx, by) = (cx, cy.clamp(ay, end));
        let d2 = (ax - bx) * (ax - bx) + (by - cy) * (by - cy);
        if nearest_rope.is_none_or(|(_, _, best)| d2 < best) {
            nearest_rope = Some((ax - cx, end - cy, d2));
        }
    }
    if let Some((dx, dy, _)) = nearest_rope {
        out[109] = (dx / ROPE_VIEW_DISTANCE).clamp(-1.0, 1.0);
        out[110] = (dy / ROPE_VIEW_DISTANCE).clamp(-1.0, 1.0);
    }

    debug_assert!(out.iter().all(|v| v.is_finite()));
    out
}

/// `1.0` when the ground runs out within [`GAP_LOOKAHEAD`] tiles of `foot_col` in `dir`
/// (`+1` right, `-1` left), `0.0` when solid ground is found first.
///
/// Out of bounds reads as solid (the level's side walls), so a body backed against the edge
/// of the world reads "ground continues" — which is true of the wall it is standing against.
fn gap_beside(level: &Level, foot_col: i32, dir: i32, foot_row: i32) -> f32 {
    for step in 1..=GAP_LOOKAHEAD {
        if level.is_solid(foot_col + dir * step, foot_row) {
            return 0.0;
        }
    }
    1.0
}

/// Vector from the body to the goal, in tiles; `(0, 0)` when there is nothing to aim at,
/// which is the honest reading of "no flag in this level" — a document loses its flag between
/// the editor's eraser and the next placement, and an agent observing one should see "nowhere
/// to go" rather than an index panic.
///
/// Signed in both axes, so the same input answers "the flag is up and to the right" and "the
/// flag is below me", and centre-to-centre because this one is a direction to steer by rather
/// than a distance to arrive at.
fn goal_delta(level: &Level, player: &PlayerState) -> (f32, f32) {
    let (cx, cy) = player.aabb().center();
    let mut best = (0.0f32, 0.0f32);
    let mut best_d2 = f32::INFINITY;
    for g in &level.goals {
        let (gx, gy) = g.center();
        let (dx, dy) = (gx - cx, gy - cy);
        let d2 = dx * dx + dy * dy;
        if d2 < best_d2 {
            best_d2 = d2;
            best = (dx, dy);
        }
    }
    best
}

/// Human-readable names for the observation vector, in order. Used by
/// `train-cli`'s `sensors` subcommand and by the snapshot test's failure output.
pub fn input_names() -> Vec<String> {
    let mut names = Vec::with_capacity(N_INPUTS);
    for dy in -PATCH_RADIUS..=PATCH_RADIUS {
        for dx in -PATCH_RADIUS..=PATCH_RADIUS {
            names.push(format!("tile[{dx:+},{dy:+}]"));
        }
    }
    names.push("enemy_dx".into());
    names.push("enemy_dy".into());
    names.push("gap_right".into());
    names.push("grounded".into());
    names.push("vx".into());
    names.push("vy".into());
    names.push("can_jump".into());
    names.push("goal_dx".into());
    names.push("goal_dy".into());
    names.push("bias".into());
    names.push("wall_dir".into());
    names.push("clinging".into());
    names.push("sprint_charge".into());
    names.push("gap_left".into());
    names.push("enemy_dir".into());
    names.push("waypoint_dx".into());
    names.push("waypoint_dy".into());
    names.push("phase_x".into());
    names.push("phase_y".into());
    names.push("clock".into());
    names.push("event_eta".into());
    names.push("event_dx".into());
    names.push("event_dy".into());
    names.push("mover_vx".into());
    names.push("mover_vy".into());
    names.push("mover_phase".into());
    names.push("ground_air".into());
    names.push("on_rope".into());
    names.push("rope_dx".into());
    names.push("rope_dy".into());
    debug_assert_eq!(names.len(), N_INPUTS);
    names
}

/// The next dangerous event the observation can name, as `(ticks, x, y)`.
///
/// The best-effort answer, not a replay of the sim: a turret's ETA is its cooldown plus the
/// shot's travel time, a vent's is the next tick of its own phase, and a ball's is how long
/// its arc takes to reach the body's row. A shot or ball that is moving *away* is not an event.
/// The point is to make "wait for the gap" expressible, not to duplicate `Sim::step`.
fn next_event(
    level: &Level,
    player: &PlayerState,
    scene: &Scene<'_>,
    ph: &Physics,
) -> Option<(f32, f32, f32)> {
    let (cx, cy) = player.aabb().center();
    let body = player.aabb();
    let mut best: Option<(f32, f32, f32)> = None;
    let mut consider = |eta: f32, x: f32, y: f32| {
        if !eta.is_finite() || eta < 0.0 || eta > 600.0 {
            return;
        }
        if best.is_none_or(|(b, _, _)| eta < b) {
            best = Some((eta, x, y));
        }
    };
    // A shot already in the air: only one travelling towards the body counts, and only if it
    // shares the body's row.
    for s in scene.shots.iter().filter(|s| s.alive) {
        let sb = s.aabb();
        if sb.bottom() < body.y - 0.35 || sb.y > body.bottom() + 0.35 || s.vx.abs() < 1e-6 {
            continue;
        }
        let towards = if s.vx > 0.0 {
            sb.center().0 < cx
        } else {
            sb.center().0 > cx
        };
        if towards {
            consider(
                ((sb.center().0 - cx) / s.vx).abs(),
                sb.center().0,
                sb.center().1,
            );
        }
    }
    // A turret that has not fired yet: cooldown plus the shot's flight time.
    for t in scene.turrets.iter().filter(|t| t.alive) {
        let tb = t.aabb();
        if tb.bottom() < body.y - 0.35 || tb.y > body.bottom() + 0.35 {
            continue;
        }
        let towards = if t.dir > 0 {
            cx > tb.right()
        } else {
            cx < tb.x
        };
        if towards {
            let dist = (cx - t.muzzle_x()).abs();
            consider(
                t.cooldown as f32 + dist / ph.shot_speed.max(1e-6),
                t.muzzle_x(),
                tb.center().1,
            );
        }
    }
    // A lava ball in flight: its arc is vertical, so the ETA is the closing distance.
    for b in scene.lava_balls.iter().filter(|b| b.alive) {
        let bb = b.aabb();
        if (bb.center().0 - cx).abs() > 0.9 {
            continue;
        }
        if b.vy < -1e-6 && bb.y > body.bottom() - 0.2 {
            consider((bb.y - body.bottom()) / -b.vy, bb.center().0, bb.center().1);
        } else if b.vy > 1e-6 && bb.bottom() < body.y + 0.2 {
            consider((body.y - bb.bottom()) / b.vy, bb.center().0, bb.center().1);
        }
    }
    // The vent directly under the body: the next ball leaves on its own phase.
    let period = ph.lava_period.max(1);
    for (i, v) in scene.vents.iter().enumerate() {
        if (v.0 as f32 + 0.5 - cx).abs() > 1.0 || cy >= v.1 as f32 || v.1 as f32 - cy > 4.5 {
            continue;
        }
        if level.is_solid(v.0, v.1 - 1) {
            continue;
        }
        let phase = (i as u32 * 37) % period;
        let eta = (phase + period - scene.frames % period) % period;
        consider(eta as f32, v.0 as f32 + 0.5, v.1 as f32);
    }
    best
}

/// The nearest moving platform's velocity and cycle phase, if one is within sight.
fn nearest_mover(player: &PlayerState, scene: &Scene<'_>) -> Option<(f32, f32, f32)> {
    let (cx, cy) = player.aabb().center();
    let mut best: Option<(f32, f32, f32, f32)> = None;
    for (i, m) in scene.movers.iter().enumerate() {
        let d = ((m.center().0 - cx).powi(2) + (m.center().1 - cy).powi(2)).sqrt();
        if d > 12.0 || best.is_some_and(|(bd, _, _, _)| d >= bd) {
            continue;
        }
        let (vx, vy) = scene.mover_vel.get(i).copied().unwrap_or((0.0, 0.0));
        let phase = scene.mover_phase.get(i).copied().unwrap_or(0.0);
        best = Some((d, vx, vy, phase));
    }
    best.map(|(_, vx, vy, phase)| (vx, vy, phase))
}

/// Vector from the body to the next landing stand along the level's move graph, falling
/// back to `goal_fallback` when the body is already on the goal's own stand or off-route.
fn waypoint_delta(
    level: &Level,
    player: &PlayerState,
    current_stand: Option<usize>,
    goal_fallback: (f32, f32),
) -> (f32, f32) {
    let (cx, cy) = player.aabb().center();
    if let Some(stand) = current_stand {
        let goal = level.goals.first().copied();
        if let Some((wx, wy)) = level.routes().next_waypoint(stand, cx, goal) {
            return (wx - cx, wy - cy);
        }
    }
    goal_fallback
}

/// One row of the action table: name, horizontal input, jump held, sprint asked for.
pub type ActionRow = (&'static str, f32, bool, bool);

/// Convenience for tools that want the action table next to the input table.
pub fn action_summary() -> Vec<ActionRow> {
    AgentAction::ALL
        .iter()
        .map(|a| (a.name(), a.input_x(), a.wants_jump(), a.wants_sprint()))
        .collect()
}
