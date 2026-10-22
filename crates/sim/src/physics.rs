//! Hand-rolled AABB-vs-tile physics and the versioned **physics bible** (§6).
//!
//! No physics engine, on purpose: simpler, fully deterministic, no cross-version
//! drift. Every number the player can feel lives in [`PHYSICS`] and is derived
//! from — or into — the jump-reachability oracle in [`crate::reach`].

use crate::hash::StateHasher;
use crate::level::{Level, Rect, Tile};
use crate::DECISION_TICKS;

/// The physics bible. Units are **tiles and ticks** (1 tick = [`crate::DT`] s).
///
/// Jump capability is derived, never hand-tuned separately (§6):
/// `h = v0²/(2g) = 2.50 tiles`, `t = 2·v0/g = 40 ticks`, `d = run_speed·t = 4.00 tiles`.
///
/// The two movement abilities added on top derive the same way, which is what keeps
/// the reachability oracle honest about them:
///
/// * **sprint** — a raised speed cap that ramps in: `d = sprint_speed·t = 6.00 tiles`,
///   and the runway to earn it is `sprint_speed²/(2·run_accel) = 1.125 tiles`.
/// * **wall jump** — a jump taken off a wall instead of the ground. Same derivation as
///   `h`, so `wall_jump_v0 = jump_v0` keeps a wall jump worth exactly one jump's height
///   and a shaft climb costs no extra height arithmetic (`wall_jump_height() == 2.50`).
///
/// Jumping itself is **edge-triggered** (a fresh press, not a held button) with a
/// [`Physics::jump_buffer_ticks`] press buffer; see [`PlayerState::tick`] for why.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Physics {
    /// Downward acceleration, tiles/tick².
    pub gravity: f32,
    /// Upward velocity imparted by a jump, tiles/tick.
    pub jump_v0: f32,
    /// Upward velocity a **spring pad** gives a body that lands on it, tiles/tick.
    ///
    /// A second rung of the jump ladder, derived the same way and for the same reason: `h =
    /// v0²/(2g)` is **4.90 tiles** and `t = 2·v0/g` is **56 ticks**, against the 2.50 tiles and
    /// 40 ticks of a fully held jump. So a pad buys roughly two jumps of height and half again
    /// the airtime of one, and all four numbers a level author needs (height, airtime, and the
    /// carry at run and sprint speed) follow from the one constant.
    ///
    /// **Only the body's vertical velocity is written**, which is what "maintaining momentum"
    /// means here: a body that arrives at run speed leaves at run speed, so the pad's carry is
    /// `vx × airtime` with the `vx` the body brought — 5.60 tiles at run speed and 8.40 at
    /// sprint — and a level can aim a pad at a landing exactly the way it aims a jump. See
    /// [`Tile::Spring`](crate::Tile::Spring).
    pub spring_v0: f32,
    /// Terminal downward velocity, tiles/tick (safety against tunnelling).
    pub max_fall: f32,
    /// Horizontal speed cap, tiles/tick (6.0 tiles/s).
    pub run_speed: f32,
    /// Horizontal speed cap while sprinting, tiles/tick (9.0 tiles/s).
    ///
    /// A sprint jump therefore covers `sprint_speed·air_time` = 6.00 tiles against the
    /// 4.00 of a running one — which is the whole point of the ability, and the reason
    /// the reachability oracle has to know about it.
    pub sprint_speed: f32,
    /// Rate the speed cap climbs toward [`Physics::sprint_speed`], tiles/tick².
    ///
    /// 0.005 gives a ten-tick ramp from `run_speed` to `sprint_speed`. The cap is the
    /// *permission*, not the speed: `run_accel` is unchanged, so the body still needs
    /// `sprint_speed/run_accel = 15` ticks (1.5 tiles) of runway to actually be moving
    /// that fast. Sprint is a decision with a runway cost, not a free upgrade.
    pub sprint_ramp: f32,
    /// Ground/air horizontal acceleration, tiles/tick².
    pub run_accel: f32,
    /// Multiplier on `run_accel` for a **grounded** reversal: a turn, rather than an
    /// acceleration.
    ///
    /// Pressing against your own momentum is the one horizontal move that has to feel
    /// immediate. Everything else in this bible is a ramp, and a ramp is right for *building*
    /// speed and wrong for *changing your mind*: at the bare `run_accel` a turn was an
    /// acceleration with a negative sign — twenty ticks (0.33 s) to cross from `run_speed`,
    /// thirty from `sprint_speed` — which is two thirds of a jump's airtime spent still
    /// travelling the way you no longer want to go. That is the mushiness of a hand on the
    /// keys, and it is also a policy's inability to stop where it means to.
    ///
    /// 2.5: a grounded turn becomes 0.025 tiles/tick², so it is **eight ticks** from run speed
    /// and twelve from sprint — short enough to be a deliberate move, long enough that the body
    /// still carries momentum through it. It is a multiplier rather than a second constant so
    /// there is one "how hard does the body push" number and this only says how much harder the
    /// push is when it is fighting the body's own motion.
    ///
    /// **Grounded only, and that is the whole design rather than a caveat.** An airborne
    /// reversal is load-bearing for this game's difficulty: a body that walks off a pit lip at
    /// running speed must not be able to press back onto the pit wall before the world runs out,
    /// because every pit in the shipped pack is two rows of nothing and *that* is what makes
    /// them lethal (`crates/sim/tests/physics.rs::walking_into_a_pit_is_still_fatal` measures
    /// it: zero cling ticks, out of the world at frame 107). Scaling the turn in the air makes
    /// the lip rescue reachable and turns the pack's pits into a wall-climbing exercise, which
    /// is a difficulty change and not a comfort setting. So the air keeps the acceleration it
    /// had, the way out of a pit stays the jump off the lip, and the turn that a player actually
    /// feels — standing on the ground, deciding to go the other way — is the one that got
    /// crisper.
    ///
    /// Accelerating from rest and pressing on in the direction already held are both unaffected
    /// too, so the runway [`Physics::runway_to_speed`] documents and every launch speed the
    /// reachability oracle derives from it are unchanged.
    pub turn_accel_scale: f32,
    /// Airborne acceleration multiplier (`< 1` = less air control).
    pub air_accel_scale: f32,
    /// Deceleration when input is neutral and grounded, tiles/tick².
    pub ground_friction: f32,
    /// Deceleration when input is neutral and airborne, tiles/tick².
    ///
    /// This is the *coast*, not the brake: it is what the body loses to the air when it
    /// stops asking to move. [`Physics::brake_friction`] is the deliberate version and
    /// applies on the ground and in the air alike — see that field for why the air half
    /// is not optional.
    pub air_friction: f32,
    /// Deceleration for the explicit `brake` action, tiles/tick², **ground and air**.
    ///
    /// Brake has to bite in the air, and that is a jump-length feature rather than a
    /// comfort one. A jump's *carry* is `speed × airtime`, and airtime is set by the
    /// jump's height, so the shortest forward hop at sprint speed still carries 2.83 tiles
    /// — a body 0.6 tiles wide cannot land on a one-tile pad two tiles away with it, and
    /// over a floor of spikes two apart, every hop in the ladder overshoots onto the next
    /// spike. Before this, `brake` in the air was *bit-identical* to `walk` (both took
    /// `air_friction`), so the one action whose whole purpose is "stop moving" was inert
    /// exactly when the body most needed it, and a level with one safe tile between
    /// hazards could only be crossed by overshooting and reversing — a 124-decision dance,
    /// and while the GA could sometimes find that earlier (`best_x` 25.4 of 44 at pop 1000)
    /// it never finished it: such a level went unsolved through 200 000 episodes at pop 500
    /// while its two-safe-tile sibling was solved in 26 generations. With the air brake the
    /// same level finishes at generation 110 at pop 1000 (FINDINGS.md finding 47), which is the
    /// point — the level was not impossible, it was a precision marathon with no short hop
    /// to spend on it.
    ///
    /// With the air brake, carry is a knob: hold the direction for some blocks and then
    /// brake for the rest of the flight, and any hop from a standing jump up to the 6.0-tile
    /// sprint jump is expressible. That is what makes a hazard *ladder* — rather than a
    /// hazard *choice* — something the agent can be asked to learn.
    pub brake_friction: f32,
    /// Multiplier applied once to upward velocity when a jump is released early.
    pub jump_cut: f32,
    /// Ticks a jump is held regardless of the action: the floor under the jump ladder.
    ///
    /// Equal to [`DECISION_TICKS`] on purpose, and that equality is the whole design. The
    /// network is queried once per decision block and its answer is held for the block, so
    /// a block is the *atom* of the agent's control. A hold shorter than a block can never
    /// be asked for, and a hold longer than every block is a hold that ignores a decision —
    /// either way the agent is given a knob it cannot turn. At one block, releasing on the
    /// next decision is the shortest jump the sim has, and each further block of hold adds
    /// height, so the agent's jump length is a ladder with a rung per decision it spends.
    pub min_jump_hold: u32,
    /// Ticks of grace after leaving a ledge during which a jump still works.
    pub coyote_ticks: u32,
    /// Ticks a fresh jump press is remembered for (100 ms).
    ///
    /// Jumping is edge-triggered, so a press that arrives a few ticks before the body
    /// lands would otherwise be thrown away — with the network deciding every 4 ticks,
    /// that turns "jump now" into a timing puzzle whose only lesson is to press early.
    /// The buffer is what lets a *deliberate* press be forgiving without letting a
    /// *held* button jump again.
    pub jump_buffer_ticks: u32,
    /// Terminal fall speed while clinging to a wall, tiles/tick (3.6 tiles/s).
    ///
    /// A tenth of `max_fall`: a wall is a place to stop and think, and the slow slide is
    /// what gives the player (and the network) time to line up the next wall jump.
    pub wall_slide_fall: f32,
    /// Upward velocity imparted by a wall jump, tiles/tick.
    ///
    /// Deliberately equal to `jump_v0`: a wall jump buys exactly one jump's height, so
    /// climbing a shaft is a rhythm rather than a compounding gain.
    pub wall_jump_v0: f32,
    /// Horizontal velocity imparted *away* from the wall by a wall jump, tiles/tick.
    pub wall_jump_push: f32,
    /// Ticks a wall jump locks out horizontal input for.
    ///
    /// The push has to be real: without the lock, a policy that keeps pressing into the
    /// wall would cancel it on the very next tick and no wall jump would ever leave the
    /// wall. At 0.06 tiles/tick for 8 ticks the body drifts 0.48 tiles out — which a
    /// one-tile-wide shaft turns into a bounce off the far wall, and a wide wall into a
    /// re-approach the body has to use its air acceleration for.
    pub wall_jump_lock: u32,
    pub player_w: f32,
    pub player_h: f32,
    pub walker_w: f32,
    pub walker_h: f32,
    /// Walker patrol speed, tiles/tick.
    pub walker_speed: f32,
    /// Upward velocity retained after stomping a walker (× `jump_v0`).
    pub stomp_bounce: f32,
    /// Maximum sub-tile jitter applied to a walker's spawn position, in tiles.
    /// See [`Walker::spawn_spread`].
    pub walker_spawn_jitter: f32,
    /// How far above a walker's top the player's previous bottom must be to stomp.
    pub enemy_stomp_tol: f32,
    /// A flyer's box.
    pub flyer_w: f32,
    pub flyer_h: f32,
    /// A flyer's patrol speed, tiles/tick.
    ///
    /// **Slower than a walker's**, and the difference is the decision rather than the feel. A
    /// walker crosses the floor a body runs along, so its speed sets how long a wait is; a flyer
    /// crosses the row a jump is *made* in, so its speed sets the width of the window a jump has
    /// to be timed in. At walker speed a flyer's window is a coin flip; at three quarters of it, a
    /// body that watches the row can still commit.
    pub flyer_speed: f32,
    /// A turret's box.
    pub turret_w: f32,
    pub turret_h: f32,
    /// Ticks between a turret's shots.
    pub turret_period: u32,
    /// A shot's box.
    pub shot_w: f32,
    pub shot_h: f32,
    /// A shot's speed along its row, tiles/tick.
    ///
    /// **Faster than a sprint**, which is the strongest promise the mechanic can make: no body,
    /// at any gear, outruns a bullet. The dodge is therefore always a *vertical* one — jump over
    /// the row, or be somewhere else when it arrives — which is the decision a turret exists to
    /// add. At 0.18 tiles/tick a shot crosses a ten-tile ledge in a second, so the body has to
    /// read the turret rather than react to the bullet.
    ///
    /// It used to be 0.08, and that number was the bug: slower than `run_speed` (0.10), so the
    /// doc's own sentence was false and a player could simply walk away from a shot. A projectile
    /// nobody has to dodge is a decoration that occasionally kills someone standing still.
    pub shot_speed: f32,
    /// How far a shot travels before it fizzles, in tiles.
    ///
    /// **A wall ends a shot; this is the backstop for a row that has none.** Eighteen tiles is a
    /// third of a sixty-tile level: long enough that a turret's row is dangerous along its whole
    /// length — the ledge, the pit beside it, the air over both — and short enough that a shot
    /// fired across an open map still ends inside the level rather than sweeping the ground the
    /// body starts on. That last half is a level-design rule as much as a physics one, and the
    /// demo level relies on it: its turret fires across a ledge and a five-tile pit and dies over
    /// the pit.
    pub shot_range: f32,
    /// A moving platform's thickness, and the speed it slides at.
    pub platform_h: f32,
    /// A moving platform's speed, in tiles per tick.
    ///
    /// **One 16-pixel art pixel per tick**, which is why the number is 0.0625 and not a round
    /// 0.05. The renderer draws at a whole-number points-per-tile scale (`Camera::scale`), so a
    /// shelf moving 1/16 of a tile per tick moves exactly `scale` screen pixels per tick at every
    /// zoom — it and the body riding it step every frame instead of holding still for two frames
    /// and then jumping one. That is the whole difference between a platform that slides and one
    /// that stutters, and the reason the speed is derived from the art's own tile rather than
    /// picked by feel. It is also a pace a person reads as a platform: 62% of a run, four tiles in
    /// about a second, slow enough to step onto and quick enough to wait for.
    ///
    /// It used to be 0.02 (half a screen pixel per tick at the usual zoom), which is the one speed
    /// at which no zoom can draw the motion smoothly.
    pub mover_speed: f32,

    // ---- the elements: water, lava, vines ----------------------------
    /// Downward speed a body in water drifts at when it is not swimming, tiles/tick.
    ///
    /// 1.2 tiles a second. Slow enough that a pool is a place to think for a moment rather than
    /// a pit, fast enough that a body which ignores the water ends up on its bottom — which is
    /// the pressure the mechanic applies: swimming is a cost measured in ticks.
    pub water_sink: f32,
    /// Upward speed a swimming body rises at while jump is held, tiles/tick.
    ///
    /// 4.2 tiles a second, roughly half a run. A body can get out of anything given time, and
    /// the time is what a pool charges for.
    pub water_swim: f32,
    /// How fast a body's vertical speed drifts to the water's own rate, tiles/tick².
    ///
    /// The water does not snap the body to its rate: a body that falls in is caught over a few
    /// ticks, which is what makes the entry read as a body entering water rather than as a body
    /// hitting a floor.
    pub water_accel: f32,
    /// The fastest a body descends at the moment it enters water, tiles/tick.
    ///
    /// A cap rather than a deceleration, because a body arriving at [`Physics::max_fall`]
    /// (0.75) would otherwise carry most of a fall's speed through the first half second and be
    /// on the bottom of a two-tile pool before the water had caught it. Water takes the fall out
    /// of an arrival in one tick — that is what water is for — and `water_accel` does the rest.
    pub water_entry: f32,
    /// Horizontal speed cap in water, tiles/tick. Half a run.
    pub water_speed: f32,
    /// Upward velocity of a **push off the surface**, tiles/tick.
    ///
    /// Pressing jump with the body near the surface is not a stroke, it is a leap: the body
    /// leaves the water with this and is ballistic from there, which is the only way a pool with
    /// a lip taller than a stroke can be left at all. `v0²/(2g)` is 2.5 tiles — the height of lip
    /// a pool may have and still be escapable — and a level author reads it off
    /// [`Physics::water_breach_height`] rather than deriving it twice.
    ///
    /// **Equal to `jump_v0`, and that is the design.** The first version gave 1.6 tiles, on the
    /// theory that water should be weaker than ground. It is not weaker, it is *slippery*: a
    /// breach has no ground to stand on, so its whole horizontal budget is the airtime above the
    /// lip, and 1.6 tiles left a body with a window about eight ticks long to drift a quarter of a
    /// tile out of the pool — a move that measured as impossible in `a_swimmer_leaves_a_pool_over_its_lip`
    /// rather than merely hard. The surface push is a jump: one height, one ladder, one number.
    pub water_jump_v0: f32,
    /// How far below the surface a press still counts as a push off it, in tiles.
    ///
    /// Without the band a swimmer would have to press on the one tick its feet were inside the
    /// tolerance of the surface, and with a band the size of the body it could breach from the
    /// bottom of a puddle. Three quarters of a tile is "at the surface" as a person means it.
    pub water_surface_reach: f32,
    /// Upward speed while climbing a vine, tiles/tick.
    pub vine_climb: f32,
    /// Downward speed while hanging on a vine without climbing, tiles/tick.
    ///
    /// Slower than the wall slide (0.06): a vine is a controlled descent, and the point of the
    /// tile is that going *down* it is a move rather than a fall.
    pub vine_slide: f32,
    /// Ticks between one lava vent's balls.
    pub lava_period: u32,
    /// Upward velocity a lava ball leaves its vent with, tiles/tick.
    ///
    /// Equal to [`Physics::jump_v0`], so a ball rises exactly as high as a jump: 2.5 tiles, which
    /// is a little over the two-row clearance a body needs to walk under one and a little under
    /// the three-tile step a body might try to hide on. The number is shared deliberately — the
    /// fountain and the jump ladder are the same geometry, so a level can be read.
    pub lava_v0: f32,
    /// Lateral drift of a lava ball, tiles/tick, sign alternating by vent index.
    ///
    /// Without it every ball falls back into the vent that threw it and the hazard is a column;
    /// with it a pool throws a curtain that lands a tile or two apart, which is what makes a row
    /// of vents a rhythm rather than a set of independent poles.
    pub lava_drift: f32,
    /// A lava ball's box.
    pub lava_w: f32,
    pub lava_h: f32,
    /// Ticks a lava ball lives for at most, after which it burns out wherever it is.
    pub lava_ticks: u32,

    // ---- the rope -----------------------------------------------------
    /// Tangential push a direction press gives a body on a rope, tiles/tick².
    ///
    /// The body pumps the swing the way a person does: not by walking (there is no ground, and
    /// a direction on a rope moves no body) but by leaning into the arc. It is smaller than
    /// [`Physics::run_accel`] — 0.008 against 0.010 — because a pendulum has time: the force is
    /// applied over a whole arc, so a gentler push is still enough to build a swing of several
    /// tiles, and a runner's acceleration would make the rope a slingshot.
    pub rope_pump: f32,
    /// How close the body's hands have to pass to a rope's line to catch it, in tiles.
    ///
    /// Measured on the rope's own axis (a vertical line at the anchor's column) against the top
    /// centre of the body, so the grab is a reach rather than a box: a body can jump past a rope
    /// without touching it, which is what makes catching one a decision. 0.55 is a little under
    /// the body's own width, so a jump that puts the hands over the line catches it and one that
    /// clears it by half a tile does not.
    pub rope_grab: f32,
    /// Share of a swing's speed that survives each tick: the air and the grip taking their cut.
    ///
    /// 0.999 is a fraction of a percent per tick, which is 6% a second — enough that a swing
    /// decays when nobody pumps it (a body hangs still in a few seconds, as a body does) and
    /// little enough that a pumped arc keeps its energy for several passes.
    pub rope_drag: f32,
    /// Shortest a catch can be, in tiles: a body's reach is not a point.
    ///
    /// The grip is the distance from the anchor to the hands at the moment of the catch, so a
    /// body that catches the line right under the anchor would be on a zero-length pendulum.
    /// Three quarters of a tile is close to the body's own height and keeps the constraint's
    /// arithmetic away from a degenerate radius.
    pub rope_min_length: f32,

    // ---- the spiked ball -------------------------------------------------
    /// The ball's hitbox: a touch smaller than a tile, so it has to *reach* a body rather than
    /// merely share its cell. The art fills the cell, so the box is what a player can see coming.
    pub spike_ball_w: f32,
    pub spike_ball_h: f32,
    /// The tangential speed the ball is woken with, tiles/tick.
    ///
    /// This is the swing's whole energy budget: `v²/(2g)` is the height it climbs, so 0.22
    /// against gravity 0.0125 lifts it 1.94 tiles — a chain-length-dependent amplitude of about
    /// 59° on the four-tile default, or 2.3 tiles to either side of the anchor. Faster than a
    /// run (0.10) at the bottom of the arc, which is what makes crossing under one a timing
    /// decision rather than a walk.
    pub spike_ball_kick: f32,
    /// Share of the ball's speed that survives each tick: a little air and a little friction.
    ///
    /// 0.9995 is 3% a second, so a triggered ball keeps sweeping for half a minute instead of
    /// hanging dead two swings later — long enough for a body that backed off to meet it again,
    /// short enough that it eventually settles.
    pub spike_ball_drag: f32,

    // ---- the belt --------------------------------------------------------
    /// How far a belt carries a body standing on it, in tiles per tick.
    ///
    /// **One art pixel a tick**, the same 1/16 the mover slides at and for the same reason: at any
    /// integer zoom the carried body moves a whole number of screen pixels per frame, so a belt
    /// reads as a moving floor rather than as a floor that stutters. 0.0625 is 3.75 tiles a
    /// second — 62% of a run — which is the one speed at which a belt is worth stepping onto: fast
    /// enough to matter to a jump's carry, slow enough that a body can still walk against it.
    pub belt_speed: f32,

    // ---- the rock --------------------------------------------------------
    /// A rock's side, in tiles. One cell, because the art is one cell.
    pub rock_size: f32,
    /// Ticks a woken rock shudders before it lets go, so a body that woke it can get clear.
    ///
    /// **This is the whole difference between a trap and an unfair one.** The first version of
    /// the falling trap was a press on a clock: it never gave a body enough time to cross, because
    /// the corridor was shut for most of every cycle and the window moved under the player. A rock
    /// that starts to shake, and then falls once, gives the one thing a clock never did — a beat
    /// between the warning and the consequence, and then a world that stays changed.
    pub rock_warn: u32,
    /// How near a body has to come, in tiles of clear space, to wake a rock.
    ///
    /// Measured from the rock's box, so a body running past at floor level wakes it about two
    /// tiles early: the warning plus the fall is longer than the run to the far side, which is
    /// what makes "keep running" an answer and not a coin flip.
    pub rock_reach: f32,
    /// How far below a rock a body can be and still wake it, in tiles.
    ///
    /// A body *above* the rock (on the bridge over it) never wakes it: the condition is that the
    /// body is under the rock's bottom edge. The depth bound is there so a rock high over a shaft
    /// is not woken by somebody walking along the bottom of a different room.
    pub rock_depth: f32,

    // ---- the crate -------------------------------------------------------
    /// A crate's width and height, in tiles: one cell.
    ///
    /// The art is one cell and the box is the art, on the spiked ball's rule: a solid a body bumps
    /// into has to be the solid it can see. It is also the size that makes a crate a *step* — one
    /// tile is a hop a body can make, two would be a wall it needs the crate to be pushed to.
    pub block_size: f32,
    /// How fast a body shoves a crate it is leaning on, tiles/tick.
    ///
    /// A run, exactly (`run_speed`): pushing is walking pace with a crate in front of you, and a
    /// body that could shove faster than it can walk would be moving a box it is not keeping up
    /// with. The crate is the load, so it sets the pace.
    pub block_push: f32,
}

pub const PHYSICS: Physics = Physics {
    gravity: 0.0125,
    jump_v0: 0.25,
    spring_v0: 0.35,
    max_fall: 0.75,
    run_speed: 0.10,
    sprint_speed: 0.15,
    sprint_ramp: 0.005,
    run_accel: 0.010,
    turn_accel_scale: 2.5,
    air_accel_scale: 0.60,
    ground_friction: 0.0150,
    air_friction: 0.0040,
    brake_friction: 0.0300,
    jump_cut: 0.45,
    min_jump_hold: DECISION_TICKS,
    coyote_ticks: 4,
    jump_buffer_ticks: 6,
    wall_slide_fall: 0.06,
    wall_jump_v0: 0.25,
    wall_jump_push: 0.06,
    wall_jump_lock: 8,
    player_w: 0.60,
    player_h: 0.90,
    walker_w: 0.60,
    walker_h: 0.60,
    walker_speed: 0.04,
    stomp_bounce: 0.85,
    enemy_stomp_tol: 0.20,
    walker_spawn_jitter: 0.20,
    flyer_w: 0.70,
    flyer_h: 0.70,
    flyer_speed: 0.03,
    turret_w: 0.80,
    turret_h: 0.80,
    // Two seconds: long enough that a turret's row is a rhythm to read rather than a wall, and
    // short enough that crossing it is a decision on every attempt.
    turret_period: 120,
    shot_w: 0.30,
    shot_h: 0.30,
    // Faster than `sprint_speed` (0.15): a bullet cannot be outrun, only dodged.
    shot_speed: 0.18,
    shot_range: 18.0,
    platform_h: 0.50,
    // One art pixel a tick (see `mover_speed`): 0.0625 = 1/16, and the value is derived rather
    // than tuned so that the shelf and its rider land on the pixel grid at any integer zoom.
    mover_speed: 0.0625,
    // 0.02 is 1.2 tiles a second: a body that stops swimming is at the bottom in about two
    // seconds in a deep pool, and never feels yanked in a shallow one.
    water_sink: 0.02,
    // 0.07 is 4.2 tiles/s up, which is 3.5x the sink rate: swimming wins, slowly.
    water_swim: 0.07,
    water_accel: 0.02,
    // A quarter of `max_fall`, and half of it again above the sink rate: the water takes a
    // terminal fall out of a body in one tick without ever pulling a swimmer down.
    water_entry: 0.10,
    // Half a run. A pool is crossed at half the pace of the floor beside it, and that is the
    // route's cost.
    water_speed: 0.06,
    // 1.6 tiles of breach: enough to leave a pool whose lip is a tile above the water, not
    // enough to leave a shaft. `water_breach_height` is this number's height.
    water_jump_v0: 0.25,
    water_surface_reach: 0.75,
    // Slower than a run (0.10): a vine is a route, not a lift.
    vine_climb: 0.06,
    vine_slide: 0.03,
    // Two and a half seconds between one vent's balls: the fountain is a rhythm to read, and
    // 150 ticks is 37.5 decision blocks, so no two visits to a vent see the same phase twice.
    lava_period: 150,
    lava_v0: 0.25,
    lava_drift: 0.015,
    lava_w: 0.30,
    lava_h: 0.30,
    // A ball's arc is ~40 ticks; 150 is the backstop for a vent that has been buried by a
    // bridge since it fired.
    lava_ticks: 150,

    // The ball is most of a tile across, so the hazard the player sees is the box that kills
    // them. `kick` and `drag` are the pendulum's own numbers; see their fields.
    spike_ball_w: 0.85,
    spike_ball_h: 0.85,
    spike_ball_kick: 0.22,
    spike_ball_drag: 0.9995,
    // The shelf's own speed, for the shelf's own reason: one art pixel a tick.
    belt_speed: 0.0625,
    // One cell, a half-second shudder, and a body two tiles away wakes it: see the fields.
    rock_size: 1.0,
    rock_warn: 30,
    rock_reach: 2.0,
    rock_depth: 6.0,
    // One cell, and a shove at walking pace: see the fields.
    block_size: 1.0,
    block_push: 0.10,
    // Under `run_accel` on purpose: a pendulum has a whole arc to be pushed along.
    rope_pump: 0.008,
    // A hand's reach, not a box: a jump that misses the line by half a tile misses it.
    rope_grab: 0.55,
    // 6% a second: a swing nobody pumps comes to rest, a pumped one keeps its energy.
    rope_drag: 0.999,
    rope_min_length: 0.75,
};

impl Default for Physics {
    fn default() -> Self {
        PHYSICS
    }
}

impl Physics {
    /// How high a push off the water's surface carries a body: `v0²/(2g)`.
    ///
    /// The one number a level author needs to decide whether a pool can be left: a lip taller
    /// than this is a pool a body cannot get out of, however long it swims.
    pub fn water_breach_height(&self) -> f32 {
        self.water_jump_v0 * self.water_jump_v0 / (2.0 * self.gravity)
    }

    /// How high a lava ball rises above its vent: `v0²/(2g)`, the same ladder as a jump.
    pub fn lava_height(&self) -> f32 {
        self.lava_v0 * self.lava_v0 / (2.0 * self.gravity)
    }

    /// Ticks a lava ball spends in the air: `2·v0/g`.
    pub fn lava_air_time_ticks(&self) -> f32 {
        2.0 * self.lava_v0 / self.gravity
    }

    /// Does a shot outrun a body moving at `speed`? The mechanic's promise, as a question.
    ///
    /// A method rather than a comparison at the call site because the claim is part of what a shot
    /// *is*: a projectile a gear can leave behind is not a threat, it is a decoration that
    /// occasionally hits somebody standing still.
    pub fn shot_outruns(&self, speed: f32) -> bool {
        self.shot_speed > speed
    }

    /// Peak height of a fully held jump: `v0²/(2g)`.
    pub fn max_jump_height(&self) -> f32 {
        self.jump_v0 * self.jump_v0 / (2.0 * self.gravity)
    }

    /// Ticks airborne on flat ground: `2·v0/g`.
    pub fn air_time_ticks(&self) -> f32 {
        2.0 * self.jump_v0 / self.gravity
    }

    /// Horizontal reach of a full-speed full-height jump: `run_speed · air_time`.
    pub fn max_jump_distance(&self) -> f32 {
        self.run_speed * self.air_time_ticks()
    }

    /// Horizontal reach of a full-speed full-height **sprint** jump.
    pub fn sprint_jump_distance(&self) -> f32 {
        self.sprint_speed * self.air_time_ticks()
    }

    /// Ticks for the speed cap to climb from `run_speed` to `sprint_speed`.
    pub fn sprint_ramp_ticks(&self) -> f32 {
        (self.sprint_speed - self.run_speed) / self.sprint_ramp
    }

    /// Runway, in tiles, needed to reach `speed` from rest under `run_accel`:
    /// `v²/(2a)`. The oracle's launch speeds are this relation, and so is the
    /// "sprint needs 1.5 tiles of run-up" the level designer is told.
    pub fn runway_to_speed(&self, speed: f32) -> f32 {
        speed * speed / (2.0 * self.run_accel)
    }

    /// Peak height a wall jump reaches: `wall_jump_v0²/(2g)`.
    pub fn wall_jump_height(&self) -> f32 {
        self.wall_jump_v0 * self.wall_jump_v0 / (2.0 * self.gravity)
    }

    /// Peak rise of a spring pad's launch: `spring_v0²/(2g)`.
    ///
    /// Here rather than in the sim for the reason every other reach in this file is: the number
    /// a level is designed against has to be the number the physics derives, not a second
    /// statement of it. The sim does not consult this function to launch a body — it writes
    /// `spring_v0` and lets the integrator do the rest — so the two can only agree, which is the
    /// same arrangement [`Physics::max_jump_height`] and the reachability oracle have.
    pub fn spring_height(&self) -> f32 {
        self.spring_v0 * self.spring_v0 / (2.0 * self.gravity)
    }

    /// Ticks a spring launch spends in the air: `2·spring_v0/g`.
    pub fn spring_air_time_ticks(&self) -> f32 {
        2.0 * self.spring_v0 / self.gravity
    }

    /// Horizontal carry of a spring launch taken at `speed`.
    ///
    /// Deliberately the same `speed × airtime` a jump is measured by, so a pad and a jump are
    /// compared in one unit rather than two. Note it is a *parameter* rather than a constant on
    /// purpose: the pad preserves whatever the body arrived with, so "how far does a spring throw
    /// me" has no single answer, and the honest form of the question is the one that names a
    /// speed. A level that needs the long one is a level that needs a sprint's runway before it.
    pub fn spring_carry(&self, speed: f32) -> f32 {
        speed * self.spring_air_time_ticks()
    }

    /// Peak height of the shortest possible jump: released the instant
    /// `min_jump_hold` expires, then `jump_cut` applied once.
    ///
    /// Mirrors the branch order in [`PlayerState::tick`] (cut check, then
    /// gravity, then move) so the number reported here is the number the sim
    /// actually produces. The reachability oracle uses the real stepper, not
    /// this function, precisely so the two can never drift apart silently.
    pub fn min_jump_height(&self) -> f32 {
        let mut vy = -self.jump_v0;
        let mut height = 0.0f32;
        let mut cut_applied = false;
        let mut hold = self.min_jump_hold;
        for _ in 0..256 {
            if hold == 0 && vy < 0.0 && !cut_applied {
                vy *= self.jump_cut;
                cut_applied = true;
            }
            hold = hold.saturating_sub(1);
            vy += self.gravity;
            if vy >= 0.0 {
                break;
            }
            height += -vy;
        }
        height
    }

    /// Horizontal reach while airborne for `ticks` at `speed`.
    pub fn distance_over(&self, speed: f32, ticks: f32) -> f32 {
        speed * ticks
    }

    /// Ticks for a **grounded** turn to bring the body from `speed` to a standstill by pressing
    /// against it: `speed / (run_accel × turn_accel_scale)`.
    ///
    /// The turn's own derivation, next to [`Physics::runway_to_speed`] which derives the same
    /// relation for accelerations, so "eight ticks from run speed" is a claim a reader can check
    /// rather than a number in a comment. The airborne figure is the same expression without the
    /// scale — and, multiplied by [`Physics::air_accel_scale`], which is the one the pit invariant
    /// above is measured against.
    pub fn turn_ticks(&self, speed: f32) -> f32 {
        speed / (self.run_accel * self.turn_accel_scale)
    }

    /// Peak height a held jump *actually* reaches under the sim's discrete Euler
    /// integration, which is lower than the continuous [`Self::max_jump_height`].
    ///
    /// The continuous value is the documented derivation (§6, §9) and stays the
    /// number the physics bible advertises; this is the number the sim delivers,
    /// so diagnostics and the reachability report quote it instead.
    pub fn discrete_max_jump_height(&self) -> f32 {
        let mut vy = -self.jump_v0;
        let mut height = 0.0f32;
        for _ in 0..1024 {
            vy += self.gravity;
            if vy >= 0.0 {
                break;
            }
            height += -vy;
        }
        height
    }
}

/// The eleven discrete macro-actions (§7.3). Argmax over network outputs.
///
/// Three deliberate properties of the ordering:
///
/// * `Run` stays at index 0, so the all-zeros genome is still the free
///   "hold right" policy (its argmax ties break on the lowest index).
/// * The table is *(direction × verb)*: run and sprint each way, and a jump arm on
///   every moving gear. Mirroring `RunJump` as `RetreatJump` closed an asymmetry the
///   old table had — there was no way to jump *left*, so a leftward jump could only ever
///   be a neutral jump carrying its momentum, and a left-facing wall could not be
///   climbed at all.
/// * **The mirrored gears are appended, not inserted.** `RetreatSprint` and
///   `RetreatSprintJump` are indices 9 and 10 rather than sitting beside `Retreat`,
///   because the indices of the eight actions before them are part of what a recorded
///   genome means: the fitness trace stores `action.index()`, the scripted action
///   sequences in the tests draw from `0..COUNT`, and a re-ordering would silently
///   change what an old number referred to. A table that *reads* as direction × verb is
///   worth less than one where every index keeps its meaning.
///
/// `Sprint` used to be a *forward* gear, on the rightward pair only, justified by
/// "progress is scored rightward-only" — and that justification died in 0.5.0, when the
/// objective became distance-to-the-flag and the flag was allowed to be anywhere at all.
/// The gear stayed, and it was not cosmetic: with no leftward sprint, the mirror of
/// `l8-long-jump` — the level that exists to teach sprint — is *unbeatable*, the
/// reachability oracle cannot express a leftward sprint take-off (it drove `SprintJump`,
/// whose input is +1, for a take-off whose launch velocity was negative), and the
/// population's fastest learned behaviour was one the game could only perform to the
/// right. Both directions now have the same two gears, which is what makes the pack's
/// difficulty a fact about the *level* rather than about which way it happens to face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum AgentAction {
    /// Accelerate right.
    Run = 0,
    /// Accelerate right, jump held.
    RunJump = 1,
    /// Accelerate right at the raised sprint cap.
    Sprint = 2,
    /// Accelerate right at the raised sprint cap, jump held.
    SprintJump = 3,
    /// Accelerate left.
    Retreat = 4,
    /// Accelerate left, jump held.
    RetreatJump = 5,
    /// Neutral input with strong deceleration.
    Brake = 6,
    /// No horizontal input, jump held.
    JumpInPlace = 7,
    /// Neutral input, no jump.
    Walk = 8,
    /// Accelerate left at the raised sprint cap.
    RetreatSprint = 9,
    /// Accelerate left at the raised sprint cap, jump held.
    RetreatSprintJump = 10,
}

impl AgentAction {
    pub const COUNT: usize = 11;
    pub const ALL: [AgentAction; 11] = [
        AgentAction::Run,
        AgentAction::RunJump,
        AgentAction::Sprint,
        AgentAction::SprintJump,
        AgentAction::Retreat,
        AgentAction::RetreatJump,
        AgentAction::Brake,
        AgentAction::JumpInPlace,
        AgentAction::Walk,
        AgentAction::RetreatSprint,
        AgentAction::RetreatSprintJump,
    ];

    /// Map a network output index to an action. Out-of-range indices clamp, so a
    /// corrupted champion cannot panic the sim.
    pub fn from_index(i: usize) -> AgentAction {
        AgentAction::ALL[i.min(AgentAction::COUNT - 1)]
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            AgentAction::Run => "run",
            AgentAction::RunJump => "run_jump",
            AgentAction::Sprint => "sprint",
            AgentAction::SprintJump => "sprint_jump",
            AgentAction::Retreat => "retreat",
            AgentAction::RetreatJump => "retreat_jump",
            AgentAction::Brake => "brake",
            AgentAction::JumpInPlace => "jump_in_place",
            AgentAction::Walk => "walk",
            AgentAction::RetreatSprint => "retreat_sprint",
            AgentAction::RetreatSprintJump => "retreat_sprint_jump",
        }
    }

    /// Horizontal input: `-1.0`, `0.0`, or `+1.0`.
    pub fn input_x(self) -> f32 {
        match self {
            AgentAction::Run
            | AgentAction::RunJump
            | AgentAction::Sprint
            | AgentAction::SprintJump => 1.0,
            AgentAction::Retreat
            | AgentAction::RetreatJump
            | AgentAction::RetreatSprint
            | AgentAction::RetreatSprintJump => -1.0,
            _ => 0.0,
        }
    }

    /// Whether the jump button is held for this action.
    ///
    /// Held, not pressed: the button being down is what keeps a full-height jump from
    /// being cut short, and per-tick control of the ascent is what the agent has instead
    /// of an analogue stick. Jumping *again* needs a fresh press — see
    /// [`PlayerState::tick`].
    pub fn wants_jump(self) -> bool {
        matches!(
            self,
            AgentAction::RunJump
                | AgentAction::SprintJump
                | AgentAction::RetreatJump
                | AgentAction::JumpInPlace
                | AgentAction::RetreatSprintJump
        )
    }

    /// Whether this action asks for the raised speed cap.
    ///
    /// A gear, not a direction: asking for it in either direction is the same request,
    /// and the two sprinting-*and*-jumping arms are the ones that have to hold it for a
    /// whole flight (see `reach::LaunchSpeed::wants_sprint`).
    pub fn wants_sprint(self) -> bool {
        matches!(
            self,
            AgentAction::Sprint
                | AgentAction::SprintJump
                | AgentAction::RetreatSprint
                | AgentAction::RetreatSprintJump
        )
    }
}

/// Per-tick contact report. Consumed by [`crate::Sim`]; never scored directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickContact {
    pub landed: bool,
    /// A jump left the ground: a ground jump, a buffered one, or a wall jump. The
    /// episode counts these, which is what the per-jump fitness term is charged on.
    pub jumped: bool,
    /// The jump in [`TickContact::jumped`] was taken **off a wall** rather than the ground.
    ///
    /// Reporting only (WP13): the fitness does not distinguish the two, but a technique archive
    /// cannot tell a wall climb from a staircase without it.
    pub wall_jumped: bool,
    pub hit_wall: bool,
    pub hit_ceiling: bool,
    pub touched_spike: bool,
    /// The body overlapped a lava cell. Its own flag rather than folded into `touched_spike`,
    /// because the two hazards answer different boxes and a reader of this report should not
    /// have to know which one killed it.
    pub touched_lava: bool,
    /// The body caught a rope this tick, or let one go.
    ///
    /// Reported separately from the jump because they are different events with different
    /// consequences: a grab spends no jump and a release is not a jump, and the replay, the
    /// sound and the pose all want to tell them apart. Nothing scores them.
    pub grabbed_rope: bool,
    pub released_rope: bool,
}

/// Bottom edge of an entity occupying tile row `ty`, in world units.
///
/// An entity at `y = 12` resting on solid tiles in row 13 has bottom `13.0`.
#[inline]
pub fn bottom_of(tile_y: i32) -> f32 {
    (tile_y + 1) as f32
}

/// Inclusive tile index range covered by `[min, min + size)`.
///
/// The epsilon keeps a body flush against a tile edge from claiming the next
/// tile over — the single most common source of jitter in AABB-vs-grid code.
///
/// **Public on purpose.** This is the project's one statement of "which tiles does
/// this body occupy", and the renderer must not have a second, subtly different
/// opinion: a body drawn one tile wider than the collision code thinks it is would
/// make a physics bug invisible instead of obvious.
#[inline]
pub fn tile_span(min: f32, size: f32) -> (i32, i32) {
    const EPS: f32 = 1.0e-4;
    (min.floor() as i32, (min + size - EPS).floor() as i32)
}

/// Inclusive row range covered by a vertical span. See [`tile_span`].
#[inline]
pub fn row_span(top: f32, height: f32) -> (i32, i32) {
    tile_span(top, height)
}

/// Ticks between a body standing on a brittle tile and that tile leaving the level.
///
/// Thirty-two, which is eight decision blocks: a body running at `run_speed` covers 3.2 tiles in
/// that time, so a cracked bridge is a span of about three tiles crossed at a run and about five
/// at a sprint. Both halves of that are the design. Short enough that a bridge is a *decision* —
/// stand still on one and you are going into the pit that appears under you — and long enough
/// that the crossing is a run rather than a reflex, since the shortest thing the agent can ask
/// for is one decision block and the whole action set is held in blocks of
/// [`crate::DECISION_TICKS`].
///
/// It is a multiple of the decision cadence for the same reason `Physics::min_jump_hold` is: a
/// timer the agent's own clock cannot divide evenly gives it a deadline it can never quite aim
/// at. Nothing about the physics depends on the number; what depends on it is whether a bridge is
/// learnable, and a bridge the population can only cross by luck is a bridge that teaches it
/// nothing.
pub const CRUMBLE_TICKS: u32 = 8 * crate::DECISION_TICKS;

/// How much of the **top** of a spike's tile is not the spike.
///
/// The sprite's top three of sixteen pixel rows are transparent, and deliberately so: a
/// spike tile drawn all the way to its ceiling stops reading as a hazard and starts
/// reading as part of the wall above it (`tools/art/terrain.py::spikes`). The kill zone
/// follows the drawing instead of the tile, so the two cannot disagree — a body that
/// clears every drawn pixel of a spike is clear of the spike. A spike is therefore lethal
/// over the lower 0.8125 tiles of its cell, which is the height the player is looking at.
///
/// The alternative, drawing the spikes up to the tile edge, was rejected: it would have
/// kept a full tile of hazard and made the *picture* the thing that changes, when the
/// picture is the only contract the player has. `REVIEW.md` records the measured cost of
/// the old rule — an invisible kill band 0.1875 tiles tall on top of every spike.
pub const SPIKE_TOP_CLEARANCE: f32 = 3.0 / 16.0;

/// The lethal box of a spike tile: the part of the cell the sprite actually covers.
///
/// Horizontally it is the whole cell — the spikes stand on the full width of their stone
/// base, and their bases reach both cell edges — and vertically it starts
/// [`SPIKE_TOP_CLEARANCE`] below the cell's top and runs to its floor. Box-versus-box
/// rather than per-pixel: the triangles taper to thin points near their apexes, and a
/// body clipping the blank air *between* two tips inside that band is still killed. That
/// is the one place this box is stricter than the drawing, and it is the honest
/// approximation — the alternative is pixel collision for one tile type.
#[inline]
pub fn spike_lethal_box(tx: i32, ty: i32) -> Rect {
    Rect::new(
        tx as f32,
        ty as f32 + SPIKE_TOP_CLEARANCE,
        1.0,
        1.0 - SPIKE_TOP_CLEARANCE,
    )
}

/// How much of the **top** of a pad's tile is not the pad.
///
/// The spring sprite leaves its first four of sixteen pixel rows transparent, for the reason the
/// spike's does: a pad drawn to its ceiling would touch the tile above it and stop reading as a
/// thing standing in a cell. The launch zone follows the drawing rather than the tile, so a body
/// that lands on a pad is landing on the pad it can see — the same rule [`SPIKE_TOP_CLEARANCE`]
/// follows for the hazard (see `crates/sim/tests/physics.rs` and `REVIEW.md` §2.12, which is where
/// the price of the alternative was measured).
///
/// A body cannot tunnel through a box this tall: the deepest a body falls in one tick is
/// `Physics::max_fall` (0.75 tiles) and the box is 0.75 tiles tall, so consecutive positions of a
/// falling body's box always overlap it. The margin is thin on purpose — a taller clearance would
/// be a bigger dead band above the pad, and a wider box would be a pad that catches a body which
/// has not reached it.
pub const SPRING_TOP_CLEARANCE: f32 = 4.0 / 16.0;

/// The live box of a spring pad: the part of the cell the sprite actually covers.
///
/// Horizontally the whole cell, because the pad's base and cap reach both edges; vertically from
/// [`SPRING_TOP_CLEARANCE`] below the cell's top down to its floor. Contrast
/// [`spike_lethal_box`], which is the same construction for the other tile a body arrives on.
#[inline]
pub fn spring_launch_box(tx: i32, ty: i32) -> Rect {
    Rect::new(
        tx as f32,
        ty as f32 + SPRING_TOP_CLEARANCE,
        1.0,
        1.0 - SPRING_TOP_CLEARANCE,
    )
}

/// Throw a body that is **descending onto** a pad under it, if there is one.
///
/// The pad writes the body's *vertical* velocity and nothing else, so the horizontal momentum the
/// body arrived with is the momentum it leaves with: a spring is a redirection of a fall, not a
/// speed the level hands out ([`Tile::Spring`](crate::Tile::Spring)).
///
/// **Only a descending body is launched** (`vy > 0`), which is the whole of what "jumps on it"
/// means and the difference between a pad and an obstacle: a body rising through the cell from
/// below, or walking across it on the ground, passes through untouched. It is also why the pad
/// needs no collision of its own — it is not solid, so a body that is not thrown falls through it
/// and is caught by whatever is underneath.
///
/// Tested against the box the *sprite* covers ([`spring_launch_box`]) rather than the whole cell,
/// for the reason the spike's kill zone is: a pad's art leaves its top quarter transparent so it
/// does not touch the ceiling, and a body that lands on the pad it can see is a body landing on the
/// pad. See that box's own note for why the height cannot be tunnelled through at
/// [`Physics::max_fall`].
///
/// **It lives beside the box and the physics because two callers must not be able to disagree about
/// it: the episode loop and the reachability oracle.** `Sim::step` runs this after the physics tick;
/// [`crate::reach`] runs it at the same point inside its canonical trajectories, which is what lets
/// the oracle answer "is this level beatable" about a level whose only way across a gap is a pad at
/// the lip. It used to drive [`PlayerState::tick`] alone — one layer below this rule — and so
/// modelled a game with no pads in it (`REVIEW.md` §1.12).
///
/// Returns whether a launch happened, which the oracle needs and the episode loop does not.
pub fn spring_launch(level: &Level, player: &mut PlayerState, ph: &Physics) -> bool {
    if player.vy <= 0.0 {
        return false;
    }
    let body = player.aabb();
    let (sx0, sx1) = tile_span(body.x, body.w);
    let (sy0, sy1) = row_span(body.y, body.h);
    for ty in sy0..=sy1 {
        for tx in sx0..=sx1 {
            if level.is_spring(tx, ty) && body.overlaps(&spring_launch_box(tx, ty)) {
                player.bounce(ph.spring_v0);
                return true;
            }
        }
    }
    false
}

/// A body's grip on a rope: the pivot it swings about, and how much line is out.
///
/// The rope is modelled as an **inextensible line with a point mass on the end** — the body —
/// rather than as a chain of linked segments. That is a deliberate choice and it is what the
/// swinging *game* is: a body on a rope is a pendulum, and what a player learns is the period of
/// the arc and the moment to let go. A segmented rope would simulate the line's own whip, which
/// nothing in the level can see and no policy can use, at the cost of a state per segment per
/// tick and a grab that has to decide which segment it caught.
///
/// **The constraint is a projection, not a spring.** Each tick the hands are integrated as a free
/// point under gravity and the pump; anything past `length` from the anchor is then moved back
/// onto the circle and the radial part of the velocity is taken away. That is what an
/// inextensible line does — it can pull but never push — and doing it as a projection rather than
/// a force is why the swing cannot explode: the body is put back on the circle every tick, so the
/// error is one tick deep no matter how fast the arc is.
///
/// The alternative, a spring with a stiffness, was not tried and does not need to be: a spring
/// stretches with speed, which would make a fast swing longer than a slow one, and the length of
/// the rope is the one number a level author chooses.
///
/// No trig anywhere. The position is carried in Cartesian coordinates and constrained by
/// normalising the offset, which is two multiplies and a square root — deterministic on every
/// build, unlike `sin`/`cos`, which is the same rule the rest of this crate's physics follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RopeGrip {
    /// The pivot: the middle of the underside of the tile the line is tied to.
    pub anchor_x: f32,
    pub anchor_y: f32,
    /// How much line is out right now, in tiles: the radius of the arc.
    pub length: f32,
    /// The rope's own length: the longest this grip may extend to.
    pub max_length: f32,
}

/// The player's body and movement state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub w: f32,
    pub h: f32,
    pub grounded: bool,
    /// Remaining greedy-jump-hold ticks (see [`Physics::min_jump_hold`]).
    jump_hold_ticks: u32,
    jump_cut_applied: bool,
    coyote_ticks: u32,
    /// Remaining ticks a fresh jump press is remembered for.
    jump_buffer: u32,
    /// Whether the jump button was held last tick, for edge detection.
    jump_held: bool,
    /// Current horizontal speed cap, ramping between `run_speed` and `sprint_speed`.
    speed_cap: f32,
    /// Side a wall blocked the body on last tick: `-1` left, `+1` right, `0` none.
    wall_dir: i8,
    /// Remaining ticks of post-wall-jump horizontal lockout.
    wall_lock: u32,
    /// The body's centre was in water this tick, so it is swimming rather than falling.
    ///
    /// Read by the window (the swim pose, the surface line's own animation) and by nothing in
    /// the physics: the medium is a function of position, recomputed at the top of every tick.
    /// It is a field because "what is the body in" is a question the *outside* asks, and one
    /// place to ask it is worth more than a second implementation of the rule.
    pub in_water: bool,
    /// The body was airborne and overlapping a vine this tick, so it is climbing.
    pub on_vine: bool,
    /// The rope this body is hanging from, when it is holding one.
    ///
    /// `Some` for as long as the grip is held, which is exactly as long as jump is held: the
    /// rope answers the same button the vine does — *use the thing you are on* — and letting go
    /// of it is how a swing ends. The body keeps whatever speed the arc had built, which is the
    /// whole reason the object exists.
    pub rope: Option<RopeGrip>,
    /// A pull-up in progress: the body has climbed a vine to the block it hangs from and is
    /// hauling itself over that block's lip.
    mantle: Option<Mantle>,
}

/// One scripted pull-up over a vine's own block.
///
/// A vine that hangs from a **ledge** is a route: the body climbs it and the oracle can leave the
/// vine sideways onto the ledge beside it. A vine that hangs from a *block* — a one-tile-thick
/// ceiling with air above it — is a dead end under the old rules: the body climbs until its head
/// meets the underside and then has nowhere to go, because the only way on is *over* the block it
/// is holding. That is the move this models: hold the climb at the top and the body hauls itself
/// up onto the block, centred on the vine's column.
///
/// Scripted rather than physical, for the same reason a spring's throw is: it is a traversal the
/// body cannot express with air control, and a script that is the same every time is what makes it
/// replayable, sweepable by the oracle and cheap to render. Collisions are ignored for its ten
/// ticks — the body is going *through* the corner it is climbing over — and it ends standing.
/// Ticks a pull-up takes: ten at 60 Hz is a sixth of a second. Long enough to read as a haul
/// over a lip, short enough that the game does not feel like it took the stick away.
pub const MANTLE_TICKS: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mantle {
    from_x: f32,
    from_y: f32,
    to_x: f32,
    to_y: f32,
    done: u32,
    total: u32,
}

/// What a body is moving through, as the physics asks it once a tick.
///
/// One enum rather than two booleans at the call sites: the three media are exclusive, and a
/// body that is both in water and touching a vine (a flooded shaft with a vine in it) has to
/// resolve to one of them. Water wins, because it is the medium that changes the *body*, and
/// the swim is the move a body in a flooded shaft needs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Medium {
    Air,
    /// The body's centre is in water: it sinks, swims, and is dragged.
    Water,
    Vine,
}

/// How far below a vine's top a body may be for a sideways press to be the way **off** it.
///
/// The last stretch of a climb is the exit: a press there carries the body out of the column and
/// onto the ledge beside the run, while it is still rising at `vine_climb` (which is why the
/// window is most of a tile — a press made on the decision cadence still has room to cross).
/// Below it the vine owns the body's x while the climb is held: see [`PlayerState::tick`].
///
/// One constant rather than a number in two places: [`crate::reach::vine_action`] presses the side
/// at exactly this height, and the physics has to answer the same question the oracle's script
/// assumes, or the route would promise an exit the body cannot take.
pub const VINE_EXIT_REACH: f32 = 0.9;

/// Whether a body whose feet are at `feet` is high enough on a vine whose top cell is `top_row`
/// to leave it sideways.
pub fn vine_exit_open(feet: f32, top_row: i32) -> bool {
    feet <= top_row as f32 + VINE_EXIT_REACH
}

/// The block a vine hangs from, when a body can stand on top of it: the anchor row, given a column
/// and a row of that column's vine.
///
/// **One statement of "is this vine a way over its own block",** shared by the sim and the oracle
/// for the reason the landing boxes are shared: a pull-up the physics performs but the sweep never
/// tries is a route the game has and the oracle denies, and that is exactly the false negative that
/// made every vine under a lid read as a dead end. A vine that hangs from a *ledge* (air above its
/// top cell) has no answer here — it is left sideways, not climbed over — and neither does a
/// two-tile-thick block, because there is no room above its anchor for a body to stand in.
///
/// `head` may be any row of the vine; the run is walked up to its top before the anchor is read.
pub fn vine_mantle_anchor(level: &Level, ph: &Physics, col: i32, head: i32) -> Option<i32> {
    if !level.is_vine(col, head) {
        return None;
    }
    // The top of the run this vine is, and the block above it.
    let mut top = head;
    while top > 0 && level.is_vine(col, top - 1) {
        top -= 1;
    }
    let anchor = top - 1;
    if !level.is_solid(col, anchor) {
        return None;
    }
    // Room to stand: every row the body would occupy above the anchor has to be free.
    let first = (anchor as f32 - ph.player_h).floor() as i32;
    if (first..anchor).any(|row| level.is_opaque(col, row)) {
        return None;
    }
    Some(anchor)
}

impl PlayerState {
    /// Place the player at the level's spawn point, at rest.
    ///
    /// `grounded` is decided by the tiles directly under the spawn box rather
    /// than left for the first tick to discover: an episode whose very first
    /// action is a jump must jump, not silently do nothing. A spawn in mid-air
    /// still has to fall (and gets `coyote_ticks == 0`).
    pub fn spawn(level: &Level, ph: &Physics) -> PlayerState {
        let (c0, c1) = tile_span(level.spawn.x, level.spawn.w);
        let row = (level.spawn.y + level.spawn.h).floor() as i32;
        let supported = (c0..=c1).any(|c| level.is_solid(c, row));
        PlayerState {
            x: level.spawn.x,
            y: level.spawn.y,
            vx: 0.0,
            vy: 0.0,
            w: ph.player_w,
            h: ph.player_h,
            grounded: supported,
            jump_hold_ticks: 0,
            jump_cut_applied: false,
            coyote_ticks: if supported { ph.coyote_ticks } else { 0 },
            jump_buffer: 0,
            jump_held: false,
            speed_cap: ph.run_speed,
            wall_dir: 0,
            wall_lock: 0,
            in_water: false,
            on_vine: false,
            rope: None,
            mantle: None,
        }
    }

    /// True while the body is hanging from a rope.
    pub fn on_rope(&self) -> bool {
        self.rope.is_some()
    }

    /// True while a pull-up is in progress. The window draws it as a climb — the rising pose, since
    /// the script moves the body upward rather than its velocity — and the simulator ignores input
    /// until it ends.
    pub fn mantling(&self) -> bool {
        self.mantle.is_some()
    }

    /// Advance a pull-up one tick. Returns what contact the tick made (a mantle presses nothing
    /// and blocks nothing), or `None` when no mantle is running.
    ///
    /// The curve is a smoothstep on both axes, so the body leaves the vine slowly, crosses the
    /// lip quickly and settles onto the block rather than snapping there — which is the whole
    /// difference between a move and a teleport.
    fn advance_mantle(&mut self) -> Option<TickContact> {
        let mantle = self.mantle.as_mut()?;
        mantle.done += 1;
        let t = (mantle.done as f32 / mantle.total as f32).clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        self.x = mantle.from_x + (mantle.to_x - mantle.from_x) * eased;
        self.y = mantle.from_y + (mantle.to_y - mantle.from_y) * eased;
        self.vx = 0.0;
        self.vy = 0.0;
        let finished = mantle.done >= mantle.total;
        let (to_y, to_x) = (mantle.to_y, mantle.to_x);
        if finished {
            // Placed rather than eased into place: the last tick of a smoothstep is within a
            // hundredth of the target, and a body that ends a move a hundredth of a tile off a
            // surface is a body the next tick has to catch.
            self.x = to_x;
            self.y = to_y;
            self.mantle = None;
            self.grounded = true;
            self.coyote_ticks = 0;
            // The climb button is still held: spend the cut and keep the press *held*, so the
            // tick after the pull-up does not launch a jump nobody asked for.
            self.jump_cut_applied = true;
            self.jump_hold_ticks = 0;
            self.jump_buffer = 0;
        }
        self.in_water = false;
        self.on_vine = false;
        // The last tick of a pull-up is a landing: the body leaves the vine and puts its weight on
        // the block. Reported through the same channel every other touchdown uses, because the
        // oracle's scripts end their trajectory on a landed contact and a mantle that reported
        // nothing would look like a body that climbed onto a block and then stood there forever
        // (see `reach::simulate_vine_climb_with`).
        Some(TickContact {
            landed: finished,
            ..TickContact::default()
        })
    }

    /// One tick of a swing: the grip owns the body's motion.
    ///
    /// The order is the physics, and it is the whole implementation: a pump (a direction press),
    /// gravity, then **the line**. The line is the object's entire contribution and it is one
    /// projection — anything past the radius is put back on the circle and the radial part of the
    /// velocity is spent against it — which is why the swing keeps its energy instead of leaking
    /// it, and why the arc cannot run away: the body is on the circle every tick, so the error is
    /// a tick deep however fast it is moving.
    ///
    /// **The grip does not change length.** Holding the button is holding the line, not hauling
    /// on it: the first version shortened the grip while jump was held, and a policy that simply
    /// held the button — which is what holding on *is* — reeled itself up to the anchor in two
    /// seconds and then hung from a three-quarter-tile pendulum with no swing left in it. A rope
    /// is a swing; the length is the one the catch found.
    ///
    /// Collisions are resolved after the constraint and **the constraint is not re-applied**: a
    /// swing that meets a wall stops at the wall, and the next tick's gravity carries it back
    /// along the arc. Re-projecting after a wall would push the body through the wall to stay on
    /// the circle, which is the one thing a swing must not do.
    fn tick_rope(
        &mut self,
        level: &Level,
        ph: &Physics,
        action: AgentAction,
        contact: &mut TickContact,
    ) {
        let Some(grip) = self.rope else { return };
        let anchor = (grip.anchor_x, grip.anchor_y);

        // The hands are the point the whole model hangs from: the body is carried by them.
        let mut hx = self.x + self.w * 0.5;
        let mut hy = self.y;

        // Pump. Horizontal on purpose — that is what leaning does — and the constraint is what
        // turns it into an arc: at the bottom of a swing a sideways push is almost all
        // tangential and the arc speeds up; at the ends of the arc it is almost all radial and
        // nothing happens. That asymmetry is the mechanic.
        self.vx += action.input_x() * ph.rope_pump;
        // Gravity, uncapped: a swing is faster than a fall, which is the point of one.
        self.vy += ph.gravity;
        // Drag — the only place the arc loses energy.
        self.vx *= ph.rope_drag;
        self.vy *= ph.rope_drag;

        hx += self.vx;
        hy += self.vy;

        // The line: pull, never push.
        let (dx, dy) = (hx - anchor.0, hy - anchor.1);
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > grip.length && dist > 1.0e-6 {
            let (ux, uy) = (dx / dist, dy / dist);
            hx = anchor.0 + ux * grip.length;
            hy = anchor.1 + uy * grip.length;
            let radial = self.vx * ux + self.vy * uy;
            if radial > 0.0 {
                self.vx -= ux * radial;
                self.vy -= uy * radial;
            }
        }

        // Place the body with its hands at the end of the line, then let the terrain answer:
        // a swing into a wall stops at it, and a swing onto a floor lets go and lands.
        let old_x = self.x;
        self.x = hx - self.w * 0.5;
        if rect_hits_opaque(level, self.aabb()) {
            self.x = old_x;
            self.vx = 0.0;
            contact.hit_wall = true;
        }
        self.y = hy;
        self.grounded = false;
        let mut holding = true;
        let (tx0, tx1) = tile_span(self.x, self.w);
        if self.vy > 0.0 {
            let row = row_span(self.y, self.h).1;
            for tx in tx0..=tx1 {
                if level.is_solid(tx, row) {
                    self.y = row as f32 - self.h;
                    self.vy = 0.0;
                    self.grounded = true;
                    self.jump_cut_applied = false;
                    holding = false;
                    contact.landed = true;
                    contact.released_rope = true;
                    break;
                }
            }
        } else if self.vy < 0.0 {
            let row = tile_span(self.y, self.h).0;
            for tx in tx0..=tx1 {
                if level.is_opaque(tx, row) {
                    self.y = (row + 1) as f32;
                    self.vy = 0.0;
                    contact.hit_ceiling = true;
                    break;
                }
            }
        }
        self.rope = if holding { Some(grip) } else { None };
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    /// Put the body down on a surface the tile grid does not hold: a moving platform's top.
    ///
    /// One method rather than a write to six fields from the sim, because "standing on
    /// something" is exactly the state [`PlayerState`] owns: the body is placed, its fall is
    /// stopped, and the one-shot jump cut is cleared so the next press is a fresh jump rather
    /// than the tail of the last one.
    pub fn land_on(&mut self, top: f32) {
        self.y = top - self.h;
        self.vy = 0.0;
        self.grounded = true;
        self.jump_cut_applied = false;
    }

    /// Move the body with the surface under it — a platform's own motion, not the body's.
    pub fn carry(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
    }

    /// Advance one tick: intent, gravity, then per-axis AABB resolution.
    /// One tick of the body.
    ///
    /// `solids` are boxes the body collides with that are **not terrain**: crates, and whatever
    /// else the pack grows that moves. They are passed rather than looked up because they are not
    /// facts about the level — a crate's position is a fact about the episode — and because the
    /// oracle deliberately models them at their authored position ([`crate::reach`]), which is a
    /// different answer from the one the sim gives. The sim hands over its live crates; the oracle
    /// hands over the file's.
    pub fn tick(
        &mut self,
        level: &Level,
        ph: &Physics,
        action: AgentAction,
        solids: &[Rect],
    ) -> TickContact {
        let mut contact = TickContact::default();

        // ---- a pull-up owns the body until it ends -----------------------
        //
        // Checked before anything else, because a mantle is a script rather than a body: the
        // input for its ten ticks is the climb that started it, collisions do not apply (the body
        // is climbing over a corner), and the medium it is in is decided by the script.
        if let Some(done) = self.advance_mantle() {
            self.jump_held = action.wants_jump();
            return done;
        }

        // ---- what is the body in? ---------------------------------------
        //
        // Asked once, from the body's **centre**, before anything moves: water is decided by
        // the centre so the surface is a line and not a band of half-states, and a vine is
        // decided by the box so a climber's whole body is on the thing it is holding. A vine
        // only counts once the body is off the ground — a body standing at the foot of one is
        // standing on the floor, with its jump intact, and grabs the vine on the way up.
        let (cx, cy) = self.aabb().center();
        let medium = if level.is_water(cx.floor() as i32, cy.floor() as i32) {
            Medium::Water
        } else if !self.grounded && self.overlaps_vine(level) {
            Medium::Vine
        } else {
            Medium::Air
        };
        let in_air_medium = matches!(medium, Medium::Air);
        let in_water = matches!(medium, Medium::Water);

        // ---- jump: edge-triggered, buffered, then cut once ---------------
        //
        // A jump needs a **fresh press**. Holding the button down jumps once; landing
        // with it still held does not jump again. That is the whole reason this changed:
        // level-triggered jumping made "hold jump" a free, perfectly periodic hop (one
        // jump per airtime, forever, measured at 2.50 jumps per 100 frames), and it put
        // no decision anywhere — the optimal flat-ground policy was to mash, and a
        // population readily settles into exactly that.
        //
        // Edge-triggering is paired with a press buffer ([`Physics::jump_buffer_ticks`])
        // so it stays *learnable*: the network decides every 4 ticks, and a press a few
        // ticks before touchdown still fires. Deliberate jumping is forgiving; mashing is
        // not free.
        //
        // A press is spent on, in order: a ground jump (or a coyote-time one), else a
        // **wall jump** off a wall the body is clinging to. The wall jump is automatic
        // in the Super Meat Boy sense — no separate action to select — because a wall is
        // a thing you are *against*, and the verb (jump) does not change.
        let want_jump = action.wants_jump();
        if want_jump && !self.jump_held {
            self.jump_buffer = ph.jump_buffer_ticks;
        }
        let clinging = self.clinging();

        // ---- the rope: gripped while the button is held --------------------
        //
        // The same button the vine answers, and for the same reason: jump is "use the thing you
        // are on". On a vine that means climb; on a rope it means **hold on**, and letting the
        // button go is how a swing ends. The grip is therefore not a press but a *state*, which
        // is what makes a rope different from a jump: the body is attached for as long as it
        // asks to be, and everything that happens in between is the arc.
        //
        // Catching one is contact, not a press: an airborne body whose **hands** (the top centre
        // of its box) pass within [`Physics::rope_grab`] of the line catches it, and one that
        // passes half a tile wide misses. That is the decision the object adds — the jump has to
        // be aimed at the rope, not merely made near it.
        if self.rope.is_some() && (!want_jump || in_water) {
            self.rope = None;
            contact.released_rope = true;
        }
        if self.rope.is_none() && !self.grounded && !in_water && want_jump {
            if let Some(grip) = level.rope_grab(self.aabb(), ph) {
                self.rope = Some(grip);
                contact.grabbed_rope = true;
            }
        }
        let swinging = self.rope.is_some();
        if swinging {
            // Nothing buffered survives a grab: a press that would have been a jump is the
            // press that took hold of the rope.
            self.jump_buffer = 0;
        }

        // ---- the vine's own block -----------------------------------------
        //
        // A vine that hangs from a *ledge* is left sideways (the climb's exit), but a vine that
        // hangs from a **block** with air above it has no sideways exit at all: the body climbs
        // until its head meets the block's underside. Holding the climb there is the pull-up —
        // the body hauls itself over that block's lip and stands on top. It is triggered from the
        // medium rather than from a new button because the climb already is the button: the one
        // press that gets a body up a vine is the one that finishes it.
        if matches!(medium, Medium::Vine) && want_jump {
            if let Some((to_x, to_y)) = self.vine_ledge(level, ph) {
                self.mantle = Some(Mantle {
                    from_x: self.x,
                    from_y: self.y,
                    to_x,
                    to_y,
                    done: 0,
                    total: MANTLE_TICKS,
                });
                self.jump_buffer = 0;
                self.jump_held = true;
                self.vx = 0.0;
                self.vy = 0.0;
                self.on_vine = true;
                return contact;
            }
        }
        // ---- the water's surface is a launch pad -------------------------
        //
        // A fresh press at the top of a pool is not a stroke, it is a **push off the water**:
        // the body leaves with `water_jump_v0` and is ballistic from there. It is the only move
        // that gets a body out of a pool with a lip, so it is the mechanic's own exit rather
        // than a flourish.
        //
        // The test is "the body is **at** a surface", not "in water", and it has to be: a
        // swimmer at the surface **bobs**, with its centre in the air and its legs in the water,
        // so a rule keyed on submersion could only be taken on the tick the centre happened to
        // be below the line — which is not a thing a person or a policy can aim at.
        // `water_surface_at` therefore looks for water under the body's columns within
        // `water_surface_reach` of its feet, which is "at the surface" as a player means it.
        //
        // A grounded body is excluded: standing on a ledge above a pool must not quietly
        // downgrade a jump to a push.
        if !self.grounded && self.jump_buffer > 0 && self.water_surface_at(level, ph).is_some() {
            self.breach(ph);
            contact.jumped = true;
        }
        match medium {
            // What is left for a press in water is a stroke, which the vertical pass below
            // applies for as long as the button is held.
            Medium::Water => {
                self.jump_buffer = self.jump_buffer.saturating_sub(1);
            }
            // A vine is climbed, not jumped: there is no ground behind the press, and a
            // buffered ground jump carried onto one would fire the moment the body reached
            // the top of the vine and turned a climb into a launch.
            Medium::Vine => self.jump_buffer = 0,
            Medium::Air => {}
        }
        if in_air_medium && !swinging && self.jump_buffer > 0 {
            if self.can_jump() {
                self.vy = -ph.jump_v0;
                self.grounded = false;
                self.coyote_ticks = 0;
                self.jump_cut_applied = false;
                self.jump_hold_ticks = ph.min_jump_hold;
                self.jump_buffer = 0;
                contact.jumped = true;
            } else if clinging {
                // Away from the wall, and locked there for a few ticks: the push has to
                // survive the next decision, or a policy pressing back into the wall
                // would cancel it and never leave.
                self.vy = -ph.wall_jump_v0;
                self.vx = -(self.wall_dir as f32) * ph.wall_jump_push;
                self.wall_dir = 0;
                self.wall_lock = ph.wall_jump_lock;
                self.jump_cut_applied = false;
                self.jump_hold_ticks = ph.min_jump_hold;
                self.jump_buffer = 0;
                contact.jumped = true;
                contact.wall_jumped = true;
            } else {
                self.jump_buffer -= 1;
            }
        }
        let holding = want_jump || self.jump_hold_ticks > 0;
        if !swinging && !holding && self.vy < 0.0 && !self.jump_cut_applied {
            self.vy *= ph.jump_cut;
            self.jump_cut_applied = true;
        }
        self.jump_hold_ticks = self.jump_hold_ticks.saturating_sub(1);

        // ---- sprint: the cap ramps, the body is clamped to it ------------
        // The cap is state rather than a per-action constant, so releasing sprint coasts
        // back down to `run_speed` instead of braking instantly, and gaining it costs the
        // runway in `Physics::runway_to_speed`.
        let target_cap = if action.wants_sprint() {
            ph.sprint_speed
        } else {
            ph.run_speed
        };
        if self.speed_cap < target_cap {
            self.speed_cap = (self.speed_cap + ph.sprint_ramp).min(target_cap);
        } else if self.speed_cap > target_cap {
            self.speed_cap = (self.speed_cap - ph.sprint_ramp).max(target_cap);
        }

        // ---- the vine owns the body's x while the climb is held --------
        //
        // A grip, not a brush. A body that arrives at a vine **at a run** is caught: its
        // horizontal speed is spent on the hold instead of carrying it past the column, and
        // below the top of the run a direction does not push it out of the column at all. That
        // is what makes a vine a route a policy can find: "hold jump and keep running" climbs,
        // where before it was thrown off after a tile and a half. That was not a theory — it is
        // exactly how the window's best agents failed a vine hanging over a lava pit (the body
        // grabbed, climbed a tile, drifted out of the column and fell in), and it is why the
        // level's route was walkable by the oracle and not by the population.
        //
        // The last [`VINE_EXIT_REACH`] tiles below the top are the exit, the same window
        // [`crate::reach::vine_action`] presses in, so stepping off onto a ledge still works.
        // Releasing the climb below that also lets a side press step off, because a body that is
        // sliding down is no longer being held up by the vine.
        let on_vine = matches!(medium, Medium::Vine);
        let held = on_vine
            && want_jump
            && self
                .vine_top_row(level)
                .is_some_and(|top| !vine_exit_open(self.y + self.h, top));
        if on_vine && (held || !self.on_vine) {
            self.vx = 0.0;
        }

        // ---- horizontal intent ------------------------------------------
        let input = action.input_x();
        if swinging {
            // The swing owns the body's motion: its x, its y, its gravity and its collisions
            // are `PlayerState::tick_rope`, and a direction on a rope is a pump rather than a
            // walk (there is no ground under the body to walk on).
        } else if held {
            // The vine has the body: a direction is not a walk while the climb is held below
            // the exit window.
        } else if self.wall_lock > 0 {
            // The wall jump owns the body's horizontal velocity for a few ticks.
            self.wall_lock -= 1;
        } else if input != 0.0 {
            let scale = if self.grounded {
                1.0
            } else {
                ph.air_accel_scale
            };
            // A grounded turn is not an acceleration. `input * vx < 0` is "pressing against
            // the way the body is already going"; on the ground that gets
            // [`Physics::turn_accel_scale`], and in the air it deliberately does not — the
            // airborne turn is what keeps the pack's pits lethal, and the field's own comment
            // has the measurement. Building speed from rest, and pressing on, stay at
            // `run_accel` either way, so every runway in the bible is unchanged.
            let accel = if self.grounded && self.vx * input < 0.0 {
                ph.run_accel * ph.turn_accel_scale
            } else {
                ph.run_accel
            };
            self.vx += input * accel * scale;
            // Water drags: the cap is the water's, not the body's gear, so a pool is crossed
            // at half a run whatever the body was doing when it went in. Sprinting through
            // water is not a thing, and the clamp is what says so.
            let cap = if in_water {
                ph.water_speed
            } else {
                self.speed_cap
            };
            self.vx = self.vx.clamp(-cap, cap);
        } else {
            // Brake is the one neutral action that is *not* a coast, and it decelerates in
            // the air as well as on the ground: see `Physics::brake_friction` for why the
            // air half is what makes a short hop expressible at all.
            let friction = if action == AgentAction::Brake {
                ph.brake_friction
            } else if self.grounded {
                ph.ground_friction
            } else {
                ph.air_friction
            };
            if self.vx > 0.0 {
                self.vx = (self.vx - friction).max(0.0);
            } else if self.vx < 0.0 {
                self.vx = (self.vx + friction).min(0.0);
            }
        }

        // ---- move x, resolve x ------------------------------------------
        //
        // A body on a rope moves as the swing moves it, so the walk is the *other* branch. The
        // two are exclusive by construction: `swinging` is `rope.is_some()`, and the rope is
        // only held while jump is down and the body is off the ground.
        if swinging {
            self.tick_rope(level, ph, action, &mut contact);
        } else {
            self.x += self.vx;
            // Crates are boxes, not cells, so they are resolved before the grid: the grid puts the
            // body flush against a wall at a cell edge, and a crate's edge is anywhere. Same rule
            // either way — the body stops where it would have entered, and its speed is spent —
            // and doing it first means a body stopped by a crate does not also run the cell test.
            if self.vx != 0.0 {
                self.resolve_solids(solids, &mut contact);
            }
            let (ty0, ty1) = row_span(self.y, self.h);
            if self.vx > 0.0 {
                let col = tile_span(self.x, self.w).1;
                for ty in ty0..=ty1 {
                    // `is_opaque`, not `is_solid`: a one-way shelf is ground from above and air
                    // from the side, so a body walking into its edge walks past it.
                    if level.is_opaque(col, ty) {
                        self.x = col as f32 - self.w;
                        self.vx = 0.0;
                        contact.hit_wall = true;
                        break;
                    }
                }
            } else if self.vx < 0.0 {
                let col = tile_span(self.x, self.w).0;
                for ty in ty0..=ty1 {
                    if level.is_opaque(col, ty) {
                        self.x = (col + 1) as f32;
                        self.vx = 0.0;
                        contact.hit_wall = true;
                        break;
                    }
                }
            }

            // ---- vertical: gravity, or the medium's own pull, then move y -----
            //
            // One match rather than three ifs, because the media are exclusive and a body that is
            // in water must not also be wall-sliding: a swimmer pressing into a pool wall would
            // otherwise be held at the wall slide's 0.06 by a rule about walls it is not on.
            match medium {
                Medium::Water => {
                    // A body already rising faster than a stroke is **ballistic**: that is the
                    // breach jump, and letting gravity have it is what makes the leap out of a
                    // pool the height `water_breach_height` promises instead of a stroke. Inside
                    // the water, the body's vertical speed drifts to the water's own rate — the
                    // sink when the button is up, the swim when it is down.
                    if self.vy < -ph.water_swim {
                        self.vy = (self.vy + ph.gravity).min(ph.max_fall);
                    } else {
                        // The fall the body arrived with is taken out of it at the surface line,
                        // in one tick, and `water_accel` does the rest. A body that has just
                        // splashed in is caught here, not by the sink rate alone.
                        self.vy = self.vy.min(ph.water_entry);
                        let target = if want_jump {
                            -ph.water_swim
                        } else {
                            ph.water_sink
                        };
                        self.vy += (target - self.vy).clamp(-ph.water_accel, ph.water_accel);
                    }
                }
                // A vine replaces gravity outright: the body hangs where it is when the button is
                // up, and rises when it is held. Snapping rather than accelerating is the grab —
                // arriving at a vine at speed catches the body on the first tick, which is the
                // difference between a vine and a ladder-shaped suggestion.
                Medium::Vine => {
                    self.vy = if want_jump {
                        -ph.vine_climb
                    } else {
                        ph.vine_slide
                    };
                }
                Medium::Air => {
                    self.vy = (self.vy + ph.gravity).min(ph.max_fall);
                    // Pressing into a wall turns a fall into a slow slide. `wall_dir != 0` after the
                    // jump block means this tick did *not* launch off the wall (a wall jump clears
                    // it), so a jump out of a slide is never slowed by the slide it just left.
                    if clinging && self.wall_dir != 0 && self.vy > ph.wall_slide_fall {
                        self.vy = ph.wall_slide_fall;
                    }
                }
            }
            let prev_bottom = self.y + self.h;
            self.y += self.vy;
            let (tx0, tx1) = tile_span(self.x, self.w);
            self.grounded = false;
            if self.vy > 0.0 {
                let row = row_span(self.y, self.h).1;
                let mut stop: Option<f32> = None;
                for tx in tx0..=tx1 {
                    if level.is_solid(tx, row) {
                        stop = Some(row as f32);
                        break;
                    }
                }
                // A crate top is a floor wherever it happens to be. The *higher* of the two
                // surfaces wins: the body stands on the first thing it reaches, and a crate
                // resting against a step is a step's worth of height above the floor.
                if let Some(top) =
                    crate_landing(solids, self.x, self.w, prev_bottom, self.y + self.h)
                {
                    stop = Some(stop.map_or(top, |t| t.min(top)));
                }
                if let Some(top) = stop {
                    self.y = top - self.h;
                    self.vy = 0.0;
                    self.grounded = true;
                    self.jump_cut_applied = false;
                    contact.landed = true;
                }
            } else if self.vy < 0.0 {
                let row = tile_span(self.y, self.h).0;
                let mut ceiling: Option<f32> = None;
                for tx in tx0..=tx1 {
                    // Rising through a one-way shelf is the move the tile exists for.
                    if level.is_opaque(tx, row) {
                        ceiling = Some((row + 1) as f32);
                        break;
                    }
                }
                if ceiling.is_none() {
                    ceiling = crate_underside(solids, self.x, self.w, prev_bottom - self.h, self.y);
                }
                if let Some(top) = ceiling {
                    self.y = top;
                    self.vy = 0.0;
                    contact.hit_ceiling = true;
                }
            }
        }

        // ---- hazard overlap ---------------------------------------------
        // The body is tested against the spike's *drawn* box, not against the tile: a
        // spike's sprite leaves the top [`SPIKE_TOP_CLEARANCE`] of its cell transparent,
        // and a body that clears every pixel of it has cleared the hazard. Killing on the
        // whole tile instead put a 0.1875-tile invisible band above every spike — see
        // `spike_lethal_box` and REVIEW.md §2.12.
        let (hx0, hx1) = tile_span(self.x, self.w);
        let (hy0, hy1) = row_span(self.y, self.h);
        let body = self.aabb();
        'hazards: for ty in hy0..=hy1 {
            for tx in hx0..=hx1 {
                if level.is_spike(tx, ty) && body.overlaps(&spike_lethal_box(tx, ty)) {
                    contact.touched_spike = true;
                    break 'hazards;
                }
                // Lava is the *cell*, not a drawing inside it: the pool fills its tile and
                // there is no transparent band at the top for a body to stand in, so the
                // whole cell is the hazard and the art fills the whole cell.
                if level.is_lava(tx, ty)
                    && body.overlaps(&Rect::new(tx as f32, ty as f32, 1.0, 1.0))
                {
                    contact.touched_lava = true;
                }
            }
        }

        // ---- wall contact memory -----------------------------------------
        // Which side a wall blocked the body on, for the *next* tick to cling to. The
        // recall is deliberately short (one tick) and requires pressing: a cling is a
        // pressed state, so letting go of the direction ends the slide on the next tick.
        self.wall_dir = if contact.hit_wall {
            if input > 0.0 {
                1
            } else if input < 0.0 {
                -1
            } else {
                0
            }
        } else {
            0
        };

        // ---- coyote time ------------------------------------------------
        if contact.jumped {
            self.coyote_ticks = 0;
        } else if self.grounded {
            self.coyote_ticks = ph.coyote_ticks;
        } else {
            self.coyote_ticks = self.coyote_ticks.saturating_sub(1);
        }

        self.jump_held = want_jump;
        self.in_water = in_water;
        self.on_vine = matches!(medium, Medium::Vine);
        contact
    }

    /// Stop the body against any crate whose box it walked into this tick.
    ///
    /// Only the direction of travel matters: a body moving right is put back at the crate's left
    /// edge and so on, which is the same statement the tile loop above makes with cells. A body
    /// with no horizontal speed is left alone even if it overlaps — a crate shoved into a standing
    /// body is the pusher's business, and teleporting the body out of it would be a second rule.
    fn resolve_solids(&mut self, solids: &[Rect], contact: &mut TickContact) {
        for s in solids {
            if !self.aabb().overlaps(s) {
                continue;
            }
            if self.vx > 0.0 {
                self.x = s.x - self.w;
            } else if self.vx < 0.0 {
                self.x = s.x + s.w;
            } else {
                continue;
            }
            self.vx = 0.0;
            contact.hit_wall = true;
        }
    }

    /// A push off the water's surface: the body leaves the water at
    /// [`Physics::water_jump_v0`], ballistic from there.
    ///
    /// Written as its own method, beside [`PlayerState::bounce`] and for the same reason: the
    /// jump's bookkeeping is five fields, and a second place that sets four of them is a place
    /// where the fifth is wrong.
    ///
    /// **It spends the jump cut, and that is the whole difference between a push and a jump.**
    /// The cut exists to let a player shorten an ascent by letting go; a swimmer *must* let go to
    /// steer, because the button is also the only horizontal input in the water. With the cut
    /// live, every breach that was steered anywhere lost 55% of its height — measured: a 2.5-tile
    /// push peaked 0.9 tiles in and could not clear a one-tile lip, which made the pool's own exit
    /// unreachable exactly when it was being used. So the water gives a fixed launch, like a
    /// spring pad gives a fixed throw: the level's number, not the button's.
    fn breach(&mut self, ph: &Physics) {
        self.vy = -ph.water_jump_v0;
        self.grounded = false;
        self.coyote_ticks = 0;
        self.jump_cut_applied = true;
        self.jump_hold_ticks = 0;
        self.jump_buffer = 0;
    }

    /// The top of the water surface the body is **at**, if any: under its columns and within
    /// [`Physics::water_surface_reach`] of its feet.
    ///
    /// Scanned down from the feet rather than up from them, because the question is "is there
    /// water just below me" — the bobbing swimmer's case — and bounded by the reach, so a body
    /// standing on a ledge a tile above a pool is not at the pool's surface.
    fn water_surface_at(&self, level: &Level, ph: &Physics) -> Option<f32> {
        let (tx0, tx1) = tile_span(self.x, self.w);
        let feet = self.y + self.h;
        let first = feet.floor() as i32;
        let last = (feet + ph.water_surface_reach).floor() as i32;
        for tx in tx0..=tx1 {
            for ty in first..=last {
                if let Some(top) = level.water_top(tx, ty) {
                    if feet <= top + ph.water_surface_reach {
                        return Some(top);
                    }
                }
            }
        }
        None
    }

    /// Where a pull-up would put the body, if one is available **right now**.
    ///
    /// Available means all of: the body is on a vine; the cell above that vine's run is solid (the
    /// block the vine hangs from); the body's head is against that block's underside; and the cell
    /// above the block is free for the body to stand in. The target is the top of that block,
    /// centred on the vine's column, so the body ends the move standing on the thing it climbed.
    ///
    /// A two-tile-thick block has no answer here and that is correct: there is no room above its
    /// anchor for a body to stand in, so the vine hanging under it is a dead end rather than a
    /// route, and pretending otherwise would put the body inside a wall.
    fn vine_ledge(&self, level: &Level, ph: &Physics) -> Option<(f32, f32)> {
        let (tx0, tx1) = tile_span(self.x, self.w);
        let head = row_span(self.y, self.h).0;
        for col in tx0..=tx1 {
            let Some(anchor) = vine_mantle_anchor(level, ph, col, head) else {
                continue;
            };
            // Pressed against the underside: the head is at the block's bottom edge, within a
            // third of a tile. Without this a body part-way up the vine would be teleported by
            // pressing climb, which is not the move.
            let underside = (anchor + 1) as f32;
            if self.y < underside - 0.05 || self.y > underside + 0.35 {
                continue;
            }
            return Some((col as f32 + (1.0 - self.w) * 0.5, anchor as f32 - self.h));
        }
        None
    }
    /// The top row of the vine run the body is holding, if it is holding one.
    ///
    /// The run is walked up from the cell the body overlaps, so a body halfway up a shaft reads
    /// the same top as a body at its foot. `None` for a body that is not on a vine, which is the
    /// question the hold rule asks before it takes the body's x away.
    fn vine_top_row(&self, level: &Level) -> Option<i32> {
        let (tx0, tx1) = tile_span(self.x, self.w);
        let (ty0, ty1) = row_span(self.y, self.h);
        let mut top: Option<i32> = None;
        for tx in tx0..=tx1 {
            for ty in ty0..=ty1 {
                if !level.is_vine(tx, ty) {
                    continue;
                }
                let mut run_top = ty;
                while run_top > 0 && level.is_vine(tx, run_top - 1) {
                    run_top -= 1;
                }
                top = Some(top.map_or(run_top, |held| held.min(run_top)));
            }
        }
        top
    }

    /// Does the body's box overlap a vine cell?
    fn overlaps_vine(&self, level: &Level) -> bool {
        let (tx0, tx1) = tile_span(self.x, self.w);
        let (ty0, ty1) = row_span(self.y, self.h);
        (ty0..=ty1).any(|ty| (tx0..=tx1).any(|tx| level.is_vine(tx, ty)))
    }

    /// True when a jump would currently be accepted.
    pub fn can_jump(&self) -> bool {
        self.grounded || self.coyote_ticks > 0
    }

    /// Throw the body upward at `v0`, **keeping its horizontal velocity**, from a spring pad.
    ///
    /// The horizontal half is what "maintaining momentum" means and it is the absence of code:
    /// this writes `vy` and `grounded` and touches nothing else, so the `vx` the body arrived
    /// with is the `vx` it leaves with. A body that reaches a pad at run speed carries the pad's
    /// whole 56 ticks of airtime at run speed, which is what [`Physics::spring_carry`] is.
    ///
    /// **It spends the jump cut, and that is not a detail.** The pad's throw is not a button: a
    /// body that arrives on a pad is not holding jump, and the cut ([`Physics::jump_cut`]) exists
    /// to shorten a *released* ascent. Left unspent, the very next tick would cut the throw to 45%
    /// of itself and a pad would deliver 0.99 tiles of rise instead of [`Physics::spring_height`]'s
    /// 4.90 — a documented number the sim did not produce, which is exactly the shape of bug the
    /// spike's kill band was (0.10.0, `REVIEW.md` §2.12). The measurement is in
    /// `a_spring_launches_a_falling_body_upward_and_keeps_its_momentum`: unspent, a pad's rise is
    /// 0.91 tiles against a jump's 2.37.
    pub fn bounce(&mut self, v0: f32) {
        self.vy = -v0;
        self.grounded = false;
        self.jump_cut_applied = true;
        self.jump_hold_ticks = 0;
    }

    /// True while the body is sliding down a wall it is pressing into.
    ///
    /// Airborne, against a wall, and not rising: the wall slide is a *fall*, so a body
    /// that is still climbing past a wall is not on it yet.
    pub fn clinging(&self) -> bool {
        !self.grounded && self.wall_dir != 0 && self.vy >= 0.0
    }

    /// Side a wall last blocked the body on: `-1` left, `+1` right, `0` none.
    pub fn wall_dir(&self) -> i8 {
        self.wall_dir
    }

    /// The horizontal speed cap the body may currently be accelerated to.
    pub fn speed_cap(&self) -> f32 {
        self.speed_cap
    }

    /// Set the speed cap directly. Used by the reachability oracle, which launches a
    /// canonical trajectory already at speed and must not have the first tick's clamp
    /// undo the launch it is trying to test.
    pub fn set_speed_cap(&mut self, cap: f32) {
        self.speed_cap = cap;
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.f32(self.x);
        h.f32(self.y);
        h.f32(self.vx);
        h.f32(self.vy);
        h.f32(self.w);
        h.f32(self.h);
        h.u8(self.grounded as u8);
        h.u32(self.jump_hold_ticks);
        h.u8(self.jump_cut_applied as u8);
        h.u32(self.coyote_ticks);
        h.u32(self.jump_buffer);
        h.u8(self.jump_held as u8);
        h.f32(self.speed_cap);
        h.i8(self.wall_dir);
        match &self.mantle {
            None => h.u8(0),
            Some(m) => {
                h.u8(1);
                h.u32(m.done);
                h.f32(m.to_x);
                h.f32(m.to_y);
            }
        }
        h.u32(self.wall_lock);
        h.u8(self.in_water as u8);
        h.u8(self.on_vine as u8);
        // The swing is state in the strongest sense: two frames that differ only in how much
        // line is out are two different arcs from here on, so the pivot and the radius are
        // part of what this state *is*.
        match &self.rope {
            None => h.u8(0),
            Some(r) => {
                h.u8(1);
                h.f32(r.anchor_x);
                h.f32(r.anchor_y);
                h.f32(r.length);
                h.f32(r.max_length);
            }
        }
    }
}

/// What a walker's brain does.
///
/// Two behaviours, one body: a patrol walks its platform and turns at the edges, and a chaser
/// heads for the player and turns only at walls. The distinction is deliberately a *mode* rather
/// than a second struct, because everything the two share — the box, the stomp, the sensor
/// slots, the art — is everything that makes a walker cheap to place, and only the one line that
/// picks a direction is different.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkerMode {
    /// Walk the platform: turn at an edge or a wall.
    Patrol,
    /// Head for the body: turn at a wall only. A chaser that walked off a ledge would kill
    /// itself on the first pit, so it stops at one — which is what makes baiting it to an edge
    /// a legitimate answer rather than a bug report.
    Chase,
}

/// A patrolling walker. Turns at platform edges and walls; never leaves its tile
/// row once settled, so patrols stay predictable and deterministic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walker {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub w: f32,
    pub h: f32,
    /// `+1` right, `-1` left.
    pub dir: i8,
    pub alive: bool,
    pub grounded: bool,
    /// Whether it patrols or hunts. Static for the episode: it is level data, not state.
    pub mode: WalkerMode,
}

impl Walker {
    /// A walker at its authored position, always facing right. Used by tooling that
    /// wants the deterministic, un-jittered pose.
    pub fn new(id: u32, rect: Rect, ph: &Physics) -> Walker {
        Walker {
            id,
            x: rect.x,
            y: rect.y,
            vx: 0.0,
            vy: 0.0,
            w: ph.walker_w,
            h: ph.walker_h,
            dir: 1,
            alive: true,
            grounded: false,
            mode: WalkerMode::Patrol,
        }
    }

    /// Spawn a walker with a per-episode phase.
    ///
    /// The initial direction and a sub-tile position offset are drawn from the
    /// episode RNG, in that fixed order. This is v1's **only** gameplay randomness,
    /// and it is load-bearing: without it every trial of the same genome would be
    /// the byte-identical episode, so `episode_seed` would change nothing and
    /// `GaConfig::trials` would average N copies of one number.
    ///
    /// An offset that would leave the body unsupported is rejected rather than
    /// clamped, so a randomized walker can never be jittered into a pit and
    /// silently deleted from the level.
    pub fn spawn_spread(
        id: u32,
        rect: Rect,
        level: &Level,
        ph: &Physics,
        rng: &mut crate::rng::Rng,
    ) -> Walker {
        let dir: i8 = if crate::rng::next_f32(rng) < 0.5 {
            -1
        } else {
            1
        };
        let offset = crate::rng::next_signed_f32(rng) * ph.walker_spawn_jitter;
        let row = row_span(rect.y, ph.walker_h).1;
        let supported = |x: f32| {
            let (c0, c1) = tile_span(x, ph.walker_w);
            (c0..=c1).any(|c| level.is_solid(c, row))
        };
        let x = if offset != 0.0 && supported(rect.x + offset) {
            rect.x + offset
        } else {
            rect.x
        };
        Walker {
            id,
            x,
            y: rect.y,
            vx: 0.0,
            vy: 0.0,
            w: ph.walker_w,
            h: ph.walker_h,
            dir,
            alive: true,
            grounded: false,
            mode: WalkerMode::Patrol,
        }
    }

    /// A chaser at its authored position.
    ///
    /// No spawn draw: a chaser's direction is decided by where the body is, so there is no
    /// phase to randomise. That also keeps a chaser level's randomness exactly where a walker
    /// level's is — the patrols' own draw — instead of adding a second source.
    pub fn chaser(id: u32, rect: Rect, ph: &Physics) -> Walker {
        Walker {
            mode: WalkerMode::Chase,
            ..Walker::new(id, rect, ph)
        }
    }

    pub fn aabb(&self, ph: &Physics) -> Rect {
        Rect::new(self.x, self.y, ph.walker_w, ph.walker_h)
    }

    /// One tick of patrol. Deterministic and input-free.
    pub fn tick(&mut self, level: &Level, ph: &Physics) {
        self.tick_against(level, None, ph);
    }

    /// One tick, with the body a chaser hunts.
    ///
    /// `target_x` is the player's left edge. It is ignored by a patrol, and a chaser without
    /// one keeps walking the way it was — which is what the patrol arm asserts, and what makes
    /// [`Walker::tick`] still a complete description of a walker-only level.
    pub fn tick_against(&mut self, level: &Level, target_x: Option<f32>, ph: &Physics) {
        if !self.alive {
            return;
        }

        // Settle onto whatever is below, so a walker authored mid-air lands.
        self.vy = (self.vy + ph.gravity).min(ph.max_fall);
        self.y += self.vy;
        self.grounded = false;
        let (tx0, tx1) = tile_span(self.x, self.w);
        if self.vy > 0.0 {
            let row = row_span(self.y, self.h).1;
            for tx in tx0..=tx1 {
                if level.is_solid(tx, row) {
                    self.y = row as f32 - self.h;
                    self.vy = 0.0;
                    self.grounded = true;
                    break;
                }
            }
        }

        // A walker that leaves the world is gone. Cannot happen in a valid level,
        // but the sim never trusts input to be sane.
        if self.y > level.height as f32 {
            self.alive = false;
            return;
        }
        if !self.grounded {
            self.vx = 0.0;
            return;
        }

        if self.mode == WalkerMode::Chase {
            if let Some(tx) = target_x {
                // The body's *left edge* against the walker's own: the two boxes are the same
                // width, so this is a centre-to-centre comparison without the half-width terms.
                self.dir = if tx < self.x { -1 } else { 1 };
            }
        }
        let dir = self.dir as f32;
        let nx = self.x + ph.walker_speed * dir;
        let (by0, by1) = row_span(self.y, self.h);
        // Leading edge, nudged 1/100 tile further so it lands in the next column.
        let lead = if self.dir > 0 {
            nx + self.w + 0.01
        } else {
            nx - 0.01
        };
        let lead_col = lead.floor() as i32;
        // Walls are `is_opaque`, not `is_solid`: a one-way shelf is not a wall, and a walker
        // meeting the edge of a bridge should walk onto it the same way the player does. Its
        // *floor* probe stays `is_solid`, because standing on the shelf is the whole point.
        let wall_ahead = (by0..=by1).any(|ty| level.is_opaque(lead_col, ty));

        // Floor probe just below the front foot.
        let foot_row = (self.y + self.h + 0.05).floor() as i32;
        let floor_ahead = level.is_solid(lead_col, foot_row);
        if wall_ahead {
            self.dir = -self.dir;
            self.vx = 0.0;
        } else if !floor_ahead {
            // A patrol turns around at a ledge; a chaser simply **stops**, and the difference
            // is the whole character of the two enemies. Turning away from the body would not
            // be hunting; stepping off would solve the level's problem for the player, which
            // is why baiting one to an edge is a legitimate answer rather than an exploit.
            self.vx = 0.0;
            if self.mode == WalkerMode::Patrol {
                self.dir = -self.dir;
            }
        } else {
            self.x = nx;
            self.vx = ph.walker_speed * dir;
        }
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.u32(self.id);
        h.f32(self.x);
        h.f32(self.y);
        h.f32(self.vx);
        h.f32(self.vy);
        h.i8(self.dir);
        h.u8(self.alive as u8);
        h.u8(self.grounded as u8);
    }
}

/// A flyer: an enemy that ignores gravity and patrols a horizontal band.
///
/// It is the pack's first air enemy, and the whole of what it means as a decision is that the
/// row a jump passes through can be occupied. It bounces off the ends of its authored span and
/// off any wall it meets, keeps its own row, and can be stomped exactly as a walker can — the
/// stomp is what keeps it a hazard rather than a wall, because a body that comes down on one
/// is still a body that chose to jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flyer {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// `+1` right, `-1` left.
    pub dir: i8,
    pub alive: bool,
    /// The two ends of its beat, in world x.
    pub min_x: f32,
    pub max_x: f32,
}

impl Flyer {
    /// A flyer at the middle of its beat, facing right. Tooling's deterministic pose.
    pub fn new(id: u32, spec: crate::level::FlyerSpec, ph: &Physics) -> Flyer {
        Flyer {
            id,
            x: spec.rect.x,
            y: spec.rect.y,
            w: ph.flyer_w,
            h: ph.flyer_h,
            dir: 1,
            alive: true,
            min_x: spec.min_x,
            max_x: spec.max_x,
        }
    }

    /// Spawn with an episode phase.
    ///
    /// **Drawn after every walker's draw**, in the same RNG stream, on purpose: a level with
    /// flyers and no walkers consumes one draw per flyer and nothing else, while a level with
    /// only walkers consumes exactly what it always did. That is what keeps every recorded
    /// walker episode in the pack bit-identical after this pass.
    pub fn spawn_spread(
        id: u32,
        spec: crate::level::FlyerSpec,
        ph: &Physics,
        rng: &mut crate::rng::Rng,
    ) -> Flyer {
        let mut flyer = Flyer::new(id, spec, ph);
        flyer.dir = if crate::rng::next_f32(rng) < 0.5 {
            -1
        } else {
            1
        };
        let span = (flyer.max_x - flyer.min_x).max(0.0);
        let phase = crate::rng::next_f32(rng) * span;
        flyer.x = (flyer.min_x + phase).min(flyer.max_x);
        flyer
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    /// One tick of patrol: no gravity, no floor, only its beat and any wall in its row.
    pub fn tick(&mut self, level: &Level, ph: &Physics) {
        if !self.alive {
            return;
        }
        let nx = self.x + ph.flyer_speed * self.dir as f32;
        let lead = if self.dir > 0 {
            nx + self.w + 0.01
        } else {
            nx - 0.01
        };
        let lead_col = lead.floor() as i32;
        let (ty0, ty1) = row_span(self.y, self.h);
        let wall = (ty0..=ty1).any(|ty| level.is_opaque(lead_col, ty));
        if wall || nx < self.min_x || nx + self.w > self.max_x {
            self.dir = -self.dir;
        } else {
            self.x = nx;
        }
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.u32(self.id);
        h.f32(self.x);
        h.f32(self.y);
        h.i8(self.dir);
        h.u8(self.alive as u8);
    }
}

/// A turret: a fixed enemy that fires along the row it stands on.
///
/// Its cooldown is the whole of its state. The first shot is staggered by its index rather than
/// fired on frame zero, because two turrets placed side by side should make a rhythm and not one
/// louder beat — and the stagger is derived from the index, so it is deterministic and costs no
/// RNG draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turret {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// `+1` fires right, `-1` fires left.
    pub dir: i8,
    pub alive: bool,
    /// Ticks until the next shot.
    pub cooldown: u32,
}

impl Turret {
    pub fn new(id: u32, spec: crate::level::TurretSpec, ph: &Physics) -> Turret {
        let period = ph.turret_period.max(1);
        Turret {
            id,
            x: spec.rect.x,
            y: spec.rect.y,
            w: ph.turret_w,
            h: ph.turret_h,
            dir: spec.dir,
            alive: true,
            cooldown: (id.wrapping_mul(37) % period).max(1),
        }
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    /// One tick. Returns whether this is the tick a shot leaves the muzzle.
    pub fn tick(&mut self, ph: &Physics) -> bool {
        if !self.alive {
            return false;
        }
        self.cooldown = self.cooldown.saturating_sub(1);
        if self.cooldown == 0 {
            self.cooldown = ph.turret_period.max(1);
            return true;
        }
        false
    }

    /// The muzzle: just outside the turret's own box, so a shot never hits its owner.
    pub fn muzzle_x(&self) -> f32 {
        if self.dir > 0 {
            self.x + self.w + 0.02
        } else {
            self.x - 0.02
        }
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.u32(self.id);
        h.u8(self.alive as u8);
        h.u32(self.cooldown);
    }
}

/// A shot in flight: a box that travels along one row until a wall or its range stops it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub vx: f32,
    /// Tiles travelled so far, which is what bounds it.
    pub travelled: f32,
    pub alive: bool,
}

impl Shot {
    pub fn from_turret(t: &Turret, ph: &Physics) -> Shot {
        Shot {
            x: if t.dir > 0 {
                t.muzzle_x()
            } else {
                t.muzzle_x() - ph.shot_w
            },
            y: t.y + (t.h - ph.shot_h) * 0.5,
            w: ph.shot_w,
            h: ph.shot_h,
            vx: ph.shot_speed * t.dir as f32,
            travelled: 0.0,
            alive: true,
        }
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    pub fn tick(&mut self, level: &Level, ph: &Physics) {
        if !self.alive {
            return;
        }
        self.x += self.vx;
        self.travelled += self.vx.abs();
        // Three things end a shot, in the order that makes each one tell: the wall it reaches
        // this tick, the level's edge (so a shot fired out of an open map does not live forever
        // off-screen), and the authored range (the backstop for a row with no wall in it).
        //
        // The lead probe is the edge of the box *in the direction of travel*, not the box's
        // centre: a bullet stops when its nose touches the masonry, which is also where the body
        // stops. The box test that follows is for the one case the lead misses — a shot that
        // spawns already inside a wall (a turret authored hard against one), which would
        // otherwise fly out of the far side of the tile.
        let lead = if self.vx > 0.0 {
            self.x + self.w + 0.01
        } else {
            self.x - 0.01
        };
        let (ty0, ty1) = row_span(self.y, self.h);
        let (bx0, bx1) = tile_span(self.x, self.w);
        let hit = (ty0..=ty1).any(|ty| level.is_opaque(lead.floor() as i32, ty))
            || (ty0..=ty1).any(|ty| (bx0..=bx1).any(|tx| level.is_opaque(tx, ty)));
        let gone = self.x + self.w < 0.0 || self.x > level.width as f32;
        if hit || gone || self.travelled > ph.shot_range {
            self.alive = false;
        }
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.f32(self.x);
        h.f32(self.y);
        h.f32(self.vx);
        h.f32(self.travelled);
        h.u8(self.alive as u8);
    }
}

/// A ball of lava, thrown up out of a vent and pulled back down by gravity.
///
/// The pack's second projectile and deliberately the opposite of the first: a [`Shot`] is
/// horizontal, fast, and ends at a wall, while a lava ball is vertical, slow, and ends where it
/// started. One is a row made dangerous; this is a *column* made dangerous, which is what a pool
/// needs — the ball is what makes the air above a lava cell a place a body has to time.
///
/// **Its whole arc is gravity**, so the geometry is the jump ladder's: `v0²/(2g)` up, `2·v0/g`
/// of airtime, and its horizontal drift is the only authored part. The ball dies when its centre
/// enters a solid cell, re-enters lava (the splash), leaves the level, or outlives
/// [`Physics::lava_ticks`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LavaBall {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub vx: f32,
    pub vy: f32,
    pub alive: bool,
    /// Ticks in flight, which bounds the one case the world does not: a ball in a shaft it can
    /// never leave.
    pub ticks: u32,
}

impl LavaBall {
    /// A ball leaving `vent` (a lava cell), with the lateral drift its index gives it.
    ///
    /// The ball starts *sitting on the vent's surface*, centred, so the first tick is already
    /// an arc rather than a ball inside the pool it came from.
    pub fn spawn(vent: (i32, i32), index: usize, ph: &Physics) -> LavaBall {
        let drift = if index % 2 == 0 {
            ph.lava_drift
        } else {
            -ph.lava_drift
        };
        LavaBall {
            x: vent.0 as f32 + 0.5 - ph.lava_w * 0.5,
            y: vent.1 as f32 - ph.lava_h,
            w: ph.lava_w,
            h: ph.lava_h,
            vx: drift,
            vy: -ph.lava_v0,
            alive: true,
            ticks: 0,
        }
    }

    /// Does vent `index` fire on physics tick `tick`?
    ///
    /// One period, one phase per vent. The stride is coprime with the period, so a run of vents
    /// fires in a rolling order rather than as a volley — which is the difference between a
    /// hazard a body can read and a wall of fire that only a lucky frame crosses.
    pub fn fires(index: usize, tick: u32, ph: &Physics) -> bool {
        let period = ph.lava_period.max(1);
        tick % period == (index as u32 * 37) % period
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    /// One tick: gravity, then the world.
    ///
    /// Tested against the **centre** rather than the box, which is the difference between a ball
    /// and a body: a ball that clips a ledge corner with its edge is a ball passing it, and a
    /// 0.3-tile ball whose box stopped at every corner would read as a thing bouncing off
    /// nothing. The box is what kills a body; the centre is what the ball is.
    pub fn tick(&mut self, level: &Level, ph: &Physics) {
        if !self.alive {
            return;
        }
        self.ticks += 1;
        self.vy = (self.vy + ph.gravity).min(ph.max_fall);
        self.x += self.vx;
        self.y += self.vy;
        let (cx, cy) = self.aabb().center();
        let cell = level.get(cx.floor() as i32, cy.floor() as i32);
        if cell.is_solid() || cell.is_lava() || self.y > level.height as f32 {
            self.alive = false;
        }
        if self.ticks >= ph.lava_ticks {
            self.alive = false;
        }
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.f32(self.x);
        h.f32(self.y);
        h.f32(self.vx);
        h.f32(self.vy);
        h.u8(self.alive as u8);
        h.u32(self.ticks);
    }
}

/// A platform that slides along one axis on a fixed cycle, carrying whatever stands on it.
///
/// **Its position is a function of the frame count, not of accumulated state.** That is the
/// design decision that makes a moving platform affordable: two runs of one seed agree without
/// the sim having to hash a phase, a platform's motion cannot drift from float accumulation,
/// and a replay at any frame can be asked where the shelf was without replaying to it. What
/// *does* need state is the body riding it, and that lives in the player, not here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mover {
    pub id: u32,
    /// The box where its travel **starts**.
    pub home: Rect,
    /// How far it was authored to travel, in tiles. Never negative.
    pub span: f32,
    /// True for a lift (travels vertically), false for a slider (travels horizontally).
    pub vertical: bool,
    /// Which way it leaves `home`: `+1` right (or up), `-1` left (or down).
    pub dir: i8,
    /// How much of `span` is actually **clear of terrain**: the shelf stops at the first wall
    /// or ceiling on its path and turns there.
    ///
    /// This is the half of a mover that the level format cannot state and the editor cannot
    /// preview, because it is a fact about the *neighbourhood* of the box rather than about
    /// the box. It is computed once, when the episode is built, from the same `is_opaque`
    /// test the player's own collision uses, so a slider that was authored through a wall
    /// is a shelf that stops at it instead of a box that passes through it — and a lift that
    /// was authored into a ceiling stops under it rather than carrying its rider into it.
    pub limit: f32,
}

impl Mover {
    /// The box at `tick`, in a triangle wave over the **clear** part of its travel.
    pub fn rect_at(&self, tick: u32, ph: &Physics) -> Rect {
        let span = self.limit.clamp(0.0, self.span.max(0.0));
        let speed = ph.mover_speed.max(1e-6);
        let half = span / speed;
        let phase = tick as f32 % (2.0 * half).max(1.0);
        let travelled = if phase <= half {
            phase * speed
        } else {
            (span - (phase - half) * speed).max(0.0)
        };
        let mut rect = self.home;
        let step = travelled * self.dir as f32;
        if self.vertical {
            rect.y -= step;
        } else {
            rect.x += step;
        }
        rect
    }

    /// Where the cycle is at `tick`, in `0..1`: `0` at the home end, `0.5` at the far end.
    ///
    /// The observation's phase slot, and the planner's "when will this deck be here" question.
    /// It is a function of the frame count, exactly as [`Mover::rect_at`] is.
    pub fn phase_at(&self, tick: u32, ph: &Physics) -> f32 {
        let span = self.limit.clamp(0.0, self.span.max(0.0));
        let speed = ph.mover_speed.max(1e-6);
        let half = span / speed;
        let period = (2.0 * half).max(1.0);
        (tick as f32 % period) / period
    }

    /// Where it moved between `tick - 1` and `tick`, which is what a rider is carried by.
    pub fn delta_at(&self, tick: u32, ph: &Physics) -> (f32, f32) {
        if tick == 0 {
            return (0.0, 0.0);
        }
        let now = self.rect_at(tick, ph);
        let before = self.rect_at(tick - 1, ph);
        (now.x - before.x, now.y - before.y)
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.u32(self.id);
        h.f32(self.home.x);
        h.f32(self.home.y);
        h.f32(self.span);
        h.u8(self.vertical as u8);
        h.i8(self.dir);
    }
}

/// A spiked ball on a chain: a pendulum that hangs still until a body comes near it.
///
/// It is simulated as a **point mass on an inextensible line**, the same model the rope uses for
/// a body's swing ([`RopeGrip`]) and for the same reasons: the constraint is a projection rather
/// than a spring, the position is carried in Cartesian coordinates, and there is no `sin`/`cos`
/// anywhere — a pendulum integrated like this is a square root and four multiplies a tick, and it
/// is deterministic on every build. The one difference is that the mass is the *hazard*, not a
/// body: nothing pumps it, nothing can grab it, and its whole state is where it is and how fast.
///
/// **Still until woken.** A ball is placed hanging straight down and does not move until a body's
/// centre comes within its authored reach. Being woken is one way — the trigger latches for the
/// episode — because a trap that re-arms the moment a body steps back is a door that closes
/// again, and because a phase that resets on proximity would make the same level play differently
/// every time the body crossed the edge of the circle.
///
/// **The first shove is away from the body.** Waking it into the body's side would be a hazard
/// that kills on the trigger tick, which is a spike with a longer animation; shoving it away
/// makes the trigger a *wind-up*, and the ball comes back through the spot the body is heading
/// for with about a second of warning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpikeBall {
    pub id: u32,
    /// The pivot: the middle of the underside of the ceiling tile.
    pub anchor_x: f32,
    pub anchor_y: f32,
    /// The radius of the arc, in tiles.
    pub length: f32,
    /// How close a body has to come to the resting ball to wake it, in tiles.
    pub reach: f32,
    /// The ball's centre, in world tiles.
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// Whether the trigger has fired. Latched for the episode.
    pub swinging: bool,
    pub w: f32,
    pub h: f32,
}

impl SpikeBall {
    /// A ball hanging still at the end of its chain.
    pub fn new(id: u32, spec: crate::level::SpikeBallSpec, ph: &Physics) -> SpikeBall {
        let (ax, ay) = spec.anchor();
        SpikeBall {
            id,
            anchor_x: ax,
            anchor_y: ay,
            length: spec.length,
            reach: spec.reach,
            x: ax,
            y: ay + spec.length,
            vx: 0.0,
            vy: 0.0,
            swinging: false,
            w: ph.spike_ball_w,
            h: ph.spike_ball_h,
        }
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x - self.w * 0.5, self.y - self.h * 0.5, self.w, self.h)
    }

    /// One tick: wake if the body is near, then move the pendulum.
    ///
    /// The trigger is tested against the **resting** ball rather than the moving one: the thing a
    /// body approaches is where the ball hangs, and a trigger that chased the ball would make the
    /// range depend on how hard it was already swinging.
    pub fn tick(&mut self, body: Rect, ph: &Physics) {
        if !self.swinging {
            let (rx, ry) = (self.anchor_x, self.anchor_y + self.length);
            let (bx, by) = body.center();
            let dx = bx - rx;
            let dy = by - ry;
            if dx * dx + dy * dy > self.reach * self.reach {
                return;
            }
            self.swinging = true;
            // Away from the body: the ball climbs the far side, then returns through the arc.
            self.vx = if bx < self.anchor_x {
                ph.spike_ball_kick
            } else {
                -ph.spike_ball_kick
            };
            self.vy = 0.0;
            return;
        }

        let drag = ph.spike_ball_drag.clamp(0.0, 1.0);
        self.vx *= drag;
        self.vy = (self.vy + ph.gravity) * drag;
        self.x += self.vx;
        self.y += self.vy;

        // The line can pull but never push: put the mass back on the circle and take the radial
        // part of its velocity away. Doing it as a projection is why the swing cannot explode —
        // the error is one tick deep however fast the arc is.
        let dx = self.x - self.anchor_x;
        let dy = self.y - self.anchor_y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > 1.0e-6 {
            let ux = dx / dist;
            let uy = dy / dist;
            self.x = self.anchor_x + ux * self.length;
            self.y = self.anchor_y + uy * self.length;
            let radial = self.vx * ux + self.vy * uy;
            self.vx -= radial * ux;
            self.vy -= radial * uy;
        }
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.u32(self.id);
        h.f32(self.x);
        h.f32(self.y);
        h.f32(self.vx);
        h.f32(self.vy);
        h.u8(self.swinging as u8);
    }
}

/// How far a platform can travel from `home` before it would overlap terrain.
///
/// Walks the path in small steps and stops at the last clear position. A quarter of a tile
/// is fine: a box is at least half a tile thick, so nothing can slip between two samples, and
/// a travel of sixteen tiles is sixty-four tests once per episode rather than per frame.
///
/// The consequence an author can rely on: **a mover never passes through a wall or a
/// ceiling, and never needs a wall on the far end of its travel** — it simply turns early.
pub fn clear_travel(home: Rect, span: f32, vertical: bool, dir: i8, level: &Level) -> f32 {
    let span = span.max(0.0);
    let step = 0.25_f32;
    let sign = if dir < 0 { -1.0 } else { 1.0 };
    let mut clear = 0.0;
    let mut travelled = 0.0;
    while travelled < span {
        travelled = (travelled + step).min(span);
        let mut probe = home;
        if vertical {
            probe.y -= travelled * sign;
        } else {
            probe.x += travelled * sign;
        }
        if rect_hits_opaque(level, probe) {
            break;
        }
        clear = travelled;
    }
    clear
}

/// Does a world-space box overlap any tile that blocks movement from the side?
///
/// `is_opaque`, not `is_solid`: a one-way shelf is not a wall, so a platform may slide
/// through one — the same rule a body follows.
/// Which way the belt under `body` carries it, if the body is standing on one.
///
/// **The tile under the body's centre**, not under either edge, and for water's reason: a body
/// straddling two belts (or a belt and a floor) is carried by the one it is *on*, and the middle
/// of its feet is the only answer that does not flicker as it crosses a boundary. The caller does
/// the "is it grounded" half — a belt only carries a body whose weight is on it.
///
/// The row is read from just **below** the feet ([`Level::tile_y`] of `y + h + 0.05`), which is the
/// same half-tile tolerance the sim uses to decide what a body is standing on: a body resting on
/// the tile has its feet exactly on the boundary, and a body a hair above it is still standing.
pub fn belt_dir_at(level: &Level, body: &Rect) -> Option<i8> {
    let row = Level::tile_y(body.y + body.h + 0.05);
    let col = Level::tile_x(body.x + body.w * 0.5);
    level.tile(col, row).and_then(Tile::belt_dir)
}

/// A **rock**: a boulder that hangs under a ceiling until a body comes near, then falls once.
///
/// **It follows gravity, and it is spent by falling.** That is the whole of it, and it is a
/// different object from the press it replaces: a press was a clock, so a corridor was shut for
/// most of every cycle and the window a body needed moved under it. A rock is *state* — it sleeps,
/// it wakes once, it falls, it lands, and it never moves again — so the answer to a rock is
/// patience and then a level that has changed, rather than a rhythm to be timed forever.
///
/// **It shudders before it lets go.** A trap that falls the tick it is woken is a trap a body
/// cannot see; [`Physics::rock_warn`] is the beat between the warning and the consequence, and it
/// is what makes running past one a decision instead of a dice roll.
///
/// **A falling rock kills; a settled one is a step.** Once it lands it is a one-tile solid like a
/// crate — a body can stand on it, and a corridor it has fallen into can be jumped. That is the
/// property the press never had, and the reason this element can be put in a level at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rock {
    pub id: u32,
    /// Where it hangs: one tile, top edge on the bottom of its row.
    pub home: Rect,
    /// Its box this frame. Equal to `home` until it is woken and released.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Vertical speed, tiles/tick. Always `>= 0`: a rock falls, and nothing lifts it.
    pub vy: f32,
    /// `true` once a body has come near. Never cleared: a rock falls once.
    pub woken: bool,
    /// Ticks left of the shudder before release. Meaningless until `woken`.
    pub warn: u32,
    /// `true` once it has come to rest. Never cleared: nothing moves it again.
    pub landed: bool,
}

impl Rock {
    /// A rock hanging at `spec`.
    pub fn new(id: u32, spec: crate::level::RockSpec) -> Rock {
        let home = spec.home();
        Rock {
            id,
            home,
            x: home.x,
            y: home.y,
            w: home.w,
            h: home.h,
            vy: 0.0,
            woken: false,
            warn: 0,
            landed: false,
        }
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    /// `true` while it is in the air under its own weight — the only time it kills.
    pub fn falling(&self) -> bool {
        self.woken && !self.landed && self.warn == 0
    }

    /// `true` while it is shuddering in its socket: the frame the view shakes it on.
    pub fn shuddering(&self) -> bool {
        self.woken && !self.landed && self.warn > 0
    }

    /// Does `body` wake it? Near in the column, and **under** the rock rather than on top of it.
    ///
    /// The vertical half is what keeps a body walking the bridge over a rock from waking it: what
    /// a rock threatens is the room below it, and that is the room its trigger is in.
    fn wakes_on(&self, body: &Rect, ph: &Physics) -> bool {
        let near =
            body.x + body.w >= self.x - ph.rock_reach && body.x <= self.x + self.w + ph.rock_reach;
        let under = body.y + body.h > self.y + self.h - 0.5;
        let close = body.y < self.y + self.h + ph.rock_depth;
        near && under && close
    }

    /// Step the rock: wake (with the shudder), fall under gravity, and stop on the first surface.
    ///
    /// `crates` are landing surfaces as well as terrain, on the crate's own rule: a rock that fell
    /// *through* a crate would be a hole in the world. Position is clamped to the surface rather
    /// than resolved incrementally because the fall is at most `max_fall` (< 1 tile) a tick, so
    /// the cell the new bottom is in is the first one it can have entered.
    pub fn tick(&mut self, body: &Rect, level: &Level, ph: &Physics, crates: &[Rect]) {
        if self.landed {
            return;
        }
        if !self.woken {
            if self.wakes_on(body, ph) {
                self.woken = true;
                self.warn = ph.rock_warn;
            }
            return;
        }
        if self.warn > 0 {
            self.warn -= 1;
            return;
        }
        let prev_bottom = self.y + self.h;
        self.vy = (self.vy + ph.gravity).min(ph.max_fall);
        self.y += self.vy;
        let bottom = self.y + self.h;
        let mut surface: Option<f32> = None;
        let row = row_span(self.y, self.h).1;
        let (x0, x1) = tile_span(self.x, self.w);
        for tx in x0..=x1 {
            if level.is_solid(tx, row) {
                let top = row as f32;
                surface = Some(surface.map_or(top, |s: f32| s.min(top)));
            }
        }
        if let Some(top) = crate_landing(crates, self.x, self.w, prev_bottom, bottom) {
            surface = Some(surface.map_or(top, |s: f32| s.min(top)));
        }
        if let Some(top) = surface {
            self.y = top - self.h;
            self.vy = 0.0;
            self.landed = true;
        }
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.f32(self.y);
        h.f32(self.vy);
        h.u8(u8::from(self.woken));
        h.u32(self.warn);
        h.u8(u8::from(self.landed));
    }
}

/// A **crate**: a one-tile solid that falls, lands and can be shoved.
///
/// The one object in the pack a body can *change*: a wall is a fact about the level and a crate is a
/// fact about where a body left it. That is why it is simulated as a body rather than a tile — a
/// tile is hashed into the document, and a moved crate is the same document with a crate somewhere
/// else.
///
/// **It has no horizontal velocity.** A shove translates it and stops; nothing carries it on. That
/// is the design decision that keeps it simple and readable: a crate with momentum would slide, need
/// friction, need to decide whether a body standing on it rides the slide, and would make "push the
/// crate to the wall" a skill of timing rather than of walking. A crate is moved, not thrown.
///
/// **Two crates do not stack.** A crate stops when the box it would move into is terrain or another
/// crate, and otherwise it moves; a row of them therefore shoves along front-first, one at a time,
/// which is what a person expects and all the simulation a crate needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Vertical speed, tiles/tick. Always `>= 0`: a crate falls, and nothing lifts it.
    pub vy: f32,
    pub grounded: bool,
}

impl Block {
    /// A crate at the position its file gives it.
    pub fn new(spec: crate::level::BlockSpec) -> Block {
        let rect = spec.rect();
        Block {
            x: rect.x,
            y: rect.y,
            w: rect.w.max(0.1),
            h: rect.h.max(0.1),
            vy: 0.0,
            grounded: false,
        }
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    /// Gravity, then the tile grid: land on the first solid row under the box.
    ///
    /// It does not resolve sideways, because nothing gives it sideways speed: the only horizontal
    /// motion a crate has is [`Block::push`], which refuses a move into terrain before it makes it.
    pub fn tick(&mut self, level: &Level, ph: &Physics) {
        self.vy = (self.vy + ph.gravity).min(ph.max_fall);
        self.grounded = false;
        self.y += self.vy;
        let (x0, x1) = tile_span(self.x, self.w);
        if self.vy >= 0.0 {
            let row = row_span(self.y, self.h).1;
            if (x0..=x1).any(|tx| level.is_solid(tx, row)) {
                self.y = row as f32 - self.h;
                self.vy = 0.0;
                self.grounded = true;
            }
        } else {
            let row = row_span(self.y, self.h).0;
            if (x0..=x1).any(|tx| level.is_opaque(tx, row)) {
                self.y = (row + 1) as f32;
                self.vy = 0.0;
            }
        }
    }

    pub(crate) fn hash_into(&self, h: &mut StateHasher) {
        h.f32(self.x);
        h.f32(self.y);
        h.f32(self.vy);
    }
}

/// Try to shove the crate at `blocks[i]` `dx` tiles sideways. Returns whether it moved.
///
/// Refused when the box it would occupy is terrain or another crate — the two things a crate
/// cannot go through — so a crate pushed at a wall stops exactly flush against it and the body
/// behind it is then stopped by the crate, which is the whole of the interaction.
///
/// A free function taking the whole list rather than a method on one crate, because the refusal
/// is a fact about the list: two crates cannot be in the same box, and a method that could not see
/// its neighbours would either allow that or need the list anyway.
pub fn push_block(level: &Level, blocks: &mut [Block], i: usize, dx: f32) -> bool {
    if dx == 0.0 || !dx.is_finite() || i >= blocks.len() {
        return false;
    }
    let mut next = blocks[i];
    next.x += dx;
    let moved = next.aabb();
    if rect_hits_opaque(level, moved) {
        return false;
    }
    if blocks
        .iter()
        .enumerate()
        .any(|(j, o)| j != i && o.aabb().overlaps(&moved))
    {
        return false;
    }
    blocks[i].x = next.x;
    true
}

/// The highest crate top a descending body's feet crossed this tick, if any.
///
/// The horizontal test is strict, so a body flush against a crate's side is *beside* it and does
/// not land on it: `tile_span` uses the same epsilon trick for the same reason. The vertical test
/// is a crossing (above the top before the move, at or below it after), not an overlap, because an
/// overlap cannot tell a body arriving on a surface from one that was already inside it.
fn crate_landing(solids: &[Rect], x: f32, w: f32, prev_bottom: f32, bottom: f32) -> Option<f32> {
    const EPS: f32 = 1.0e-3;
    let mut best: Option<f32> = None;
    for s in solids {
        if x + w <= s.x + EPS || x >= s.x + s.w - EPS {
            continue;
        }
        if prev_bottom <= s.y + EPS && bottom >= s.y {
            best = Some(best.map_or(s.y, |b: f32| b.min(s.y)));
        }
    }
    best
}

/// The underside of the crate a rising body's head crossed this tick, if any: the mirror of
/// [`crate_landing`], so a jump under a crate is stopped by it instead of passing through it.
fn crate_underside(solids: &[Rect], x: f32, w: f32, prev_top: f32, top: f32) -> Option<f32> {
    const EPS: f32 = 1.0e-3;
    let mut best: Option<f32> = None;
    for s in solids {
        if x + w <= s.x + EPS || x >= s.x + s.w - EPS {
            continue;
        }
        let underside = s.y + s.h;
        if prev_top >= underside - EPS && top <= underside {
            best = Some(best.map_or(underside, |b: f32| b.max(underside)));
        }
    }
    best
}

pub(crate) fn rect_hits_opaque(level: &Level, rect: Rect) -> bool {
    let (x0, x1) = tile_span(rect.x, rect.w);
    let (y0, y1) = row_span(rect.y, rect.h);
    (y0..=y1).any(|ty| (x0..=x1).any(|tx| level.is_opaque(tx, ty)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physics_bible_derived_quantities_match_the_plan() {
        let p = &PHYSICS;
        assert!((p.max_jump_height() - 2.5).abs() < 1e-4, "h");
        assert!((p.air_time_ticks() - 40.0).abs() < 1e-4, "t");
        assert!((p.max_jump_distance() - 4.0).abs() < 1e-4, "d");
        // A tap jump must be meaningfully shorter than a held jump, or the action
        // space loses a real degree of freedom.
        let min = p.min_jump_height();
        assert!(
            min > 1.0 && min < p.max_jump_height(),
            "min jump {min} should be between 1 tile and the 2.5 tile max"
        );
    }

    /// The abilities are not new numbers to hand-tune; they are the same three
    /// derivations with the speed and velocity they name. If someone retunes
    /// `sprint_speed` or `wall_jump_v0` and these fail, the reachability oracle and the
    /// FINDINGS.md's tables are the things that silently stopped being true.
    #[test]
    fn sprint_and_wall_derive_from_the_same_constants() {
        let p = &PHYSICS;
        // Sprint buys 50% more jump distance: 6.00 tiles against 4.00.
        assert!(
            (p.sprint_jump_distance() - 6.0).abs() < 1e-4,
            "sprint jump distance {} should be sprint_speed * air_time",
            p.sprint_jump_distance()
        );
        assert!(p.sprint_jump_distance() > p.max_jump_distance());
        // And costs runway: v²/(2a). Half a tile to run, 1.125 to sprint.
        assert!((p.runway_to_speed(p.run_speed) - 0.5).abs() < 1e-4);
        assert!((p.runway_to_speed(p.sprint_speed) - 1.125).abs() < 1e-4);
        assert!(
            (p.sprint_ramp_ticks() - 10.0).abs() < 1e-4,
            "the cap should reach sprint speed in ten ticks, got {}",
            p.sprint_ramp_ticks()
        );
        // A wall jump is worth exactly one jump's height, so climbing is a rhythm.
        assert!((p.wall_jump_v0 - p.jump_v0).abs() < 1e-6);
        assert!((p.wall_jump_height() - p.max_jump_height()).abs() < 1e-4);
        // The slide is slow enough to think during, and slower than a plain fall.
        assert!(p.wall_slide_fall > 0.0 && p.wall_slide_fall < p.max_fall * 0.2);
        // The press buffer has to outlast a decision plus a tick of slack, or an
        // edge-triggered jump could be pressed and dropped between two decisions.
        assert!(
            p.jump_buffer_ticks > DECISION_TICKS,
            "buffer {} must cover a decision block of {DECISION_TICKS}",
            p.jump_buffer_ticks
        );
        // And the hold floor is exactly one decision block: see the jump-ladder test.
        assert_eq!(
            p.min_jump_hold, DECISION_TICKS,
            "min_jump_hold has to be one decision block, or the shortest jump the agent \
             can ask for is not a jump it can ask for"
        );
    }

    /// The apex of a jump the agent holds for `blocks` decision blocks, then releases.
    ///
    /// This is the whole of the agent's control over how high it jumps: it picks a hold
    /// *duration* in units of the decision cadence, because the network is asked once per
    /// block and the action it picks is held for the whole of it. A jump that ignores a
    /// block of hold is a jump the agent cannot aim.
    fn held_jump_apex(blocks: u32) -> f32 {
        let level = crate::level::Level::starter("jump-ladder", 20, 10);
        let ph = &PHYSICS;
        let mut player = PlayerState::spawn(&level, ph);
        let start_bottom = player.y + player.h;
        let mut apex = 0.0f32;
        for t in 0..200u32 {
            let held = t < blocks * DECISION_TICKS;
            let action = if held {
                AgentAction::RunJump
            } else {
                AgentAction::Run
            };
            player.tick(&level, ph, action, &[]);
            apex = apex.max(start_bottom - (player.y + player.h));
            if t > ph.min_jump_hold && player.grounded {
                break;
            }
        }
        apex
    }

    /// **Every decision block of hold buys height, and the shortest jump is short enough to
    /// clear one tile.**
    ///
    /// The action space gives the agent no analogue stick: the whole of its control over a
    /// jump's height is how many blocks it keeps holding, and the whole of its control over
    /// the *length* follows from that (a shorter air time at the same speed is a shorter
    /// jump). So "one more block of hold" has to mean "more height", one block at a time.
    ///
    /// This is the pin for finding 46. The ladder used to be
    /// `{1.70, 1.70, 2.09, 2.31, 2.38}` — holding a jump for one block and for two produced
    /// the *same* jump, because `min_jump_hold` outlasted a whole decision block, so the
    /// first release the agent could ask for was ignored. A population watching that hill
    /// has no gradient on it, and a level needing a hop between two spikes has no gear
    /// below 1.70 tiles.
    #[test]
    fn every_extra_decision_block_of_hold_buys_height() {
        let heights: Vec<f32> = (1..=5).map(held_jump_apex).collect();
        for (i, pair) in heights.windows(2).enumerate() {
            let (lower, higher) = (pair[0], pair[1]);
            assert!(
                higher > lower + 0.05,
                "holding for {} block(s) jumps {lower:.3} tiles and {} block(s) jumps \
                 {higher:.3}: block {} of hold buys nothing, so the agent cannot aim this \
                 jump. The whole ladder is {heights:?}",
                i + 1,
                i + 2,
                i + 2
            );
        }
        // The floor of the ladder is the gear a spike field is jumped in: a jump that cannot
        // clear one tile of hazard is not a short jump, it is a failed one.
        assert!(
            heights[0] > 1.0 && heights[0] < 1.5,
            "the shortest jump is {:.3} tiles; it has to clear a one-tile spike (1.0) and \
             still be a hop rather than a leap",
            heights[0]
        );
        // And the top of it is the documented full jump.
        assert!(
            (heights[4] - PHYSICS.discrete_max_jump_height()).abs() < 0.01,
            "five blocks of hold should reach the full {} tiles, got {:.3}",
            PHYSICS.discrete_max_jump_height(),
            heights[4]
        );
    }

    /// **A platform moves one art pixel a tick.**
    ///
    /// The renderer's scale is a whole number of points per 16-pixel tile (`view::Camera::scale`),
    /// so a speed of 1/16 tile per tick is exactly `scale` screen pixels per tick at *every* zoom:
    /// the shelf and the body riding it advance together on the pixel grid instead of stepping one
    /// pixel every other frame. That is the whole reason the number is what it is, so the relation
    /// is pinned here rather than left as a comment beside a literal — 0.02 was the speed at which
    /// no zoom could draw the motion smoothly.
    #[test]
    fn a_platform_moves_one_art_pixel_a_tick() {
        let pixels = PHYSICS.mover_speed * 16.0;
        assert!(
            (pixels - 1.0).abs() < 1e-6,
            "a shelf should move one 16-pixel art pixel a tick, moves {pixels}"
        );
        // And a shelf is neither a lift that throws you nor a wait that tests the player: a
        // four-tile slide takes 64 ticks, about a second.
        assert!((4.0 / PHYSICS.mover_speed - 64.0).abs() < 1e-3);
    }

    /// **A shot cannot be outrun, so its dodge is vertical.**
    ///
    /// The doc on `shot_speed` made this claim for two releases while the number was slower than a
    /// run — a promise the physics did not keep, and one a player discovers by simply walking away
    /// from a bullet. The two relations are the mechanic: above the fastest gear the body has, and
    /// below the terminal fall so the shot is still a thing the eye can track.
    #[test]
    fn a_shot_is_faster_than_a_sprint() {
        for gear in [PHYSICS.run_speed, PHYSICS.sprint_speed] {
            assert!(
                PHYSICS.shot_outruns(gear),
                "a shot at {} is outrun by a body at {gear}",
                PHYSICS.shot_speed
            );
        }
        // And a shelf is slower than a shot, which is what makes the turret's row a deadline
        // rather than a wall: the ride across it is what the bullet is timed against.
        assert!(PHYSICS.shot_outruns(PHYSICS.mover_speed));
    }

    #[test]
    fn tile_span_does_not_claim_the_neighbouring_tile() {
        assert_eq!(tile_span(5.0, 0.6), (5, 5));
        assert_eq!(tile_span(5.4, 0.6), (5, 5));
        assert_eq!(tile_span(5.4001, 0.6), (5, 6));
        assert_eq!(tile_span(-0.6, 0.6), (-1, -1));
    }

    #[test]
    fn bottom_of_places_entities_on_the_row_below() {
        assert_eq!(bottom_of(12), 13.0);
        assert_eq!(bottom_of(0), 1.0);
    }

    #[test]
    fn actions_cover_the_documented_space() {
        assert_eq!(AgentAction::ALL.len(), AgentAction::COUNT);
        for (i, a) in AgentAction::ALL.iter().enumerate() {
            assert_eq!(a.index(), i);
            assert_eq!(AgentAction::from_index(i), *a);
        }
        // Out-of-range never panics: it clamps to the last row of the table, which is what
        // a corrupted champion gets instead of an index panic in the middle of a tick.
        assert_eq!(
            AgentAction::from_index(99),
            AgentAction::ALL[AgentAction::COUNT - 1]
        );
        // `run` is index 0, so the zero genome is still the free hold-right policy.
        assert_eq!(AgentAction::ALL[0], AgentAction::Run);
    }

    /// The table is (direction × verb) with **no holes and no preferred direction**: the
    /// two moving directions offer the same gears, and nothing neutral asks for one.
    ///
    /// A hole here is a capability the network cannot express. The table has had two: no
    /// jump-left (which made left-facing walls unclimbable) and no sprint-left (which made
    /// the *mirror of `l8-long-jump`* unbeatable — see the type doc). This test is the
    /// guard that keeps the count symmetric rather than the two directions drifting apart
    /// again, and it is written as a comparison between the directions so that a future
    /// gear has to be added twice.
    #[test]
    fn the_two_moving_directions_offer_the_same_gears() {
        // Each moving direction's gears, in table order, as `(jumps, sprints)`.
        let right: Vec<_> = AgentAction::ALL
            .iter()
            .filter(|a| a.input_x() == 1.0)
            .map(|a| (a.wants_jump(), a.wants_sprint()))
            .collect();
        let left: Vec<_> = AgentAction::ALL
            .iter()
            .filter(|a| a.input_x() == -1.0)
            .map(|a| (a.wants_jump(), a.wants_sprint()))
            .collect();
        assert_eq!(
            right, left,
            "the directions must offer the same gears, in the same order: right {right:?} \
             vs left {left:?}"
        );
        // Each direction has exactly one running jump and exactly one sprinting jump —
        // the two arms a climb and a long leap are built from.
        for (dir, moves) in [("right", &right), ("left", &left)] {
            for (what, want_jump, want_sprint) in
                [("running jump", true, false), ("sprint jump", true, true)]
            {
                let n = moves
                    .iter()
                    .filter(|(j, s)| *j == want_jump && *s == want_sprint)
                    .count();
                assert_eq!(n, 1, "{dir} needs exactly one {what}, found {n}");
            }
        }
        // Nothing neutral asks for the raised cap: a sprint is a gear of a *direction*.
        for a in AgentAction::ALL.iter() {
            if a.input_x() == 0.0 {
                assert!(
                    !a.wants_sprint(),
                    "{} has no direction to sprint in",
                    a.name()
                );
            }
        }
    }

    /// The gears are *different speeds*, not two names for one action: on flat ground a
    /// sprint crosses more ground than a run, in either direction. Without this, a table
    /// that looks symmetric could still be one gear wearing two labels.
    #[test]
    fn a_sprint_covers_more_ground_than_a_run_in_both_directions() {
        let mut level = crate::Level::starter("gears", 40, 8);
        for tx in 0..level.width {
            level.set_tile(tx, 7, crate::Tile::Solid);
        }
        let travelled = |level: &crate::Level, action: AgentAction| -> f32 {
            let mut player = PlayerState::spawn(level, &PHYSICS);
            player.set_speed_cap(PHYSICS.sprint_speed);
            let x0 = player.x;
            for _ in 0..40 {
                player.tick(level, &PHYSICS, action, &[]);
            }
            (player.x - x0).abs()
        };
        for (run, sprint) in [
            (AgentAction::Run, AgentAction::Sprint),
            (AgentAction::Retreat, AgentAction::RetreatSprint),
        ] {
            // From the middle of the runway, so each direction gets the same room: a body
            // spawned at the starter level's left edge has about one tile to retreat into,
            // and the leftward leg of this comparison would measure the wall instead of
            // the gear. Twenty tiles either way is three times what 40 ticks needs.
            level.set_spawn_tile(20, 6);
            let (r, s) = (travelled(&level, run), travelled(&level, sprint));
            assert!(
                s > r * 1.2,
                "{} covers {s:.2} tiles and {} covers {r:.2}: the mirrored gear is not a \
                 second speed",
                sprint.name(),
                run.name()
            );
        }
    }
}
