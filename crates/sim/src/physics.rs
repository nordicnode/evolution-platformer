//! Hand-rolled AABB-vs-tile physics and the versioned **physics bible** (§6).
//!
//! No physics engine, on purpose: simpler, fully deterministic, no cross-version
//! drift. Every number the player can feel lives in [`PHYSICS`] and is derived
//! from — or into — the jump-reachability oracle in [`crate::reach`].

use crate::hash::StateHasher;
use crate::level::{Level, Rect};
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
}

pub const PHYSICS: Physics = Physics {
    gravity: 0.0125,
    jump_v0: 0.25,
    max_fall: 0.75,
    run_speed: 0.10,
    sprint_speed: 0.15,
    sprint_ramp: 0.005,
    run_accel: 0.010,
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
};

impl Default for Physics {
    fn default() -> Self {
        PHYSICS
    }
}

impl Physics {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub hit_wall: bool,
    pub hit_ceiling: bool,
    pub touched_spike: bool,
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
        }
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    /// Advance one tick: intent, gravity, then per-axis AABB resolution.
    pub fn tick(
        &mut self,
        level: &Level,
        ph: &Physics,
        action: AgentAction,
    ) -> TickContact {
        let mut contact = TickContact::default();

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
        if self.jump_buffer > 0 {
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
            } else {
                self.jump_buffer -= 1;
            }
        }
        let holding = want_jump || self.jump_hold_ticks > 0;
        if !holding && self.vy < 0.0 && !self.jump_cut_applied {
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

        // ---- horizontal intent ------------------------------------------
        let input = action.input_x();
        if self.wall_lock > 0 {
            // The wall jump owns the body's horizontal velocity for a few ticks.
            self.wall_lock -= 1;
        } else if input != 0.0 {
            let scale = if self.grounded { 1.0 } else { ph.air_accel_scale };
            self.vx += input * ph.run_accel * scale;
            self.vx = self.vx.clamp(-self.speed_cap, self.speed_cap);
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
        self.x += self.vx;
        let (ty0, ty1) = row_span(self.y, self.h);
        if self.vx > 0.0 {
            let col = tile_span(self.x, self.w).1;
            for ty in ty0..=ty1 {
                if level.is_solid(col, ty) {
                    self.x = col as f32 - self.w;
                    self.vx = 0.0;
                    contact.hit_wall = true;
                    break;
                }
            }
        } else if self.vx < 0.0 {
            let col = tile_span(self.x, self.w).0;
            for ty in ty0..=ty1 {
                if level.is_solid(col, ty) {
                    self.x = (col + 1) as f32;
                    self.vx = 0.0;
                    contact.hit_wall = true;
                    break;
                }
            }
        }

        // ---- gravity, wall slide, move y, resolve y ----------------------
        self.vy = (self.vy + ph.gravity).min(ph.max_fall);
        // Pressing into a wall turns a fall into a slow slide. `wall_dir != 0` after the
        // jump block means this tick did *not* launch off the wall (a wall jump clears it),
        // so a jump out of a slide is never slowed by the slide it just left.
        if clinging && self.wall_dir != 0 && self.vy > ph.wall_slide_fall {
            self.vy = ph.wall_slide_fall;
        }
        self.y += self.vy;
        let (tx0, tx1) = tile_span(self.x, self.w);
        self.grounded = false;
        if self.vy > 0.0 {
            let row = row_span(self.y, self.h).1;
            for tx in tx0..=tx1 {
                if level.is_solid(tx, row) {
                    self.y = row as f32 - self.h;
                    self.vy = 0.0;
                    self.grounded = true;
                    self.jump_cut_applied = false;
                    contact.landed = true;
                    break;
                }
            }
        } else if self.vy < 0.0 {
            let row = tile_span(self.y, self.h).0;
            for tx in tx0..=tx1 {
                if level.is_solid(tx, row) {
                    self.y = (row + 1) as f32;
                    self.vy = 0.0;
                    contact.hit_ceiling = true;
                    break;
                }
            }
        }

        // ---- hazard overlap ---------------------------------------------
        let (hx0, hx1) = tile_span(self.x, self.w);
        let (hy0, hy1) = row_span(self.y, self.h);
        'hazards: for ty in hy0..=hy1 {
            for tx in hx0..=hx1 {
                if level.is_spike(tx, ty) {
                    contact.touched_spike = true;
                    break 'hazards;
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
        contact
    }

    /// True when a jump would currently be accepted.
    pub fn can_jump(&self) -> bool {
        self.grounded || self.coyote_ticks > 0
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
        h.u32(self.wall_lock);
    }
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
        let dir: i8 = if crate::rng::next_f32(rng) < 0.5 { -1 } else { 1 };
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
        }
    }

    pub fn aabb(&self, ph: &Physics) -> Rect {
        Rect::new(self.x, self.y, ph.walker_w, ph.walker_h)
    }

    /// One tick of patrol. Deterministic and input-free.
    pub fn tick(&mut self, level: &Level, ph: &Physics) {
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
        let wall_ahead = (by0..=by1).any(|ty| level.is_solid(lead_col, ty));

        // Floor probe just below the front foot.
        let foot_row = (self.y + self.h + 0.05).floor() as i32;
        let floor_ahead = level.is_solid(lead_col, foot_row);

        if wall_ahead || !floor_ahead {
            self.dir = -self.dir;
            self.vx = 0.0;
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
            player.tick(&level, ph, action);
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
            for (what, want_jump, want_sprint) in [
                ("running jump", true, false),
                ("sprint jump", true, true),
            ] {
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
                assert!(!a.wants_sprint(), "{} has no direction to sprint in", a.name());
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
                player.tick(level, &PHYSICS, action);
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
