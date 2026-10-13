//! Hand-rolled AABB-vs-tile physics and the versioned **physics bible** (§6).
//!
//! No physics engine, on purpose: simpler, fully deterministic, no cross-version
//! drift. Every number the player can feel lives in [`PHYSICS`] and is derived
//! from — or into — the jump-reachability oracle in [`crate::reach`].

use crate::hash::StateHasher;
use crate::level::{Level, Rect};

/// The physics bible. Units are **tiles and ticks** (1 tick = [`crate::DT`] s).
///
/// Jump capability is derived, never hand-tuned separately (§6):
/// `h = v0²/(2g) = 2.50 tiles`, `t = 2·v0/g = 40 ticks`, `d = run_speed·t = 4.00 tiles`.
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
    /// Ground/air horizontal acceleration, tiles/tick².
    pub run_accel: f32,
    /// Airborne acceleration multiplier (`< 1` = less air control).
    pub air_accel_scale: f32,
    /// Deceleration when input is neutral and grounded, tiles/tick².
    pub ground_friction: f32,
    /// Deceleration when input is neutral and airborne, tiles/tick².
    pub air_friction: f32,
    /// Deceleration for the explicit `brake` action, tiles/tick².
    pub brake_friction: f32,
    /// Multiplier applied once to upward velocity when a jump is released early.
    pub jump_cut: f32,
    /// Ticks a jump is held regardless of the action, so a 4-tick action repeat
    /// cannot accidentally shrink every jump to a hop.
    pub min_jump_hold: u32,
    /// Ticks of grace after leaving a ledge during which a jump still works.
    pub coyote_ticks: u32,
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
    run_accel: 0.010,
    air_accel_scale: 0.60,
    ground_friction: 0.0150,
    air_friction: 0.0040,
    brake_friction: 0.0300,
    jump_cut: 0.45,
    min_jump_hold: 8,
    coyote_ticks: 4,
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

/// The six discrete macro-actions (§7.3). Argmax over network outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentAction {
    /// Accelerate right.
    Run = 0,
    /// Accelerate right, jump held.
    RunJump = 1,
    /// Accelerate left.
    Retreat = 2,
    /// Neutral input with strong deceleration.
    Brake = 3,
    /// No horizontal input, jump held.
    JumpInPlace = 4,
    /// Neutral input, no jump.
    Walk = 5,
}

impl AgentAction {
    pub const COUNT: usize = 6;
    pub const ALL: [AgentAction; 6] = [
        AgentAction::Run,
        AgentAction::RunJump,
        AgentAction::Retreat,
        AgentAction::Brake,
        AgentAction::JumpInPlace,
        AgentAction::Walk,
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
            AgentAction::Retreat => "retreat",
            AgentAction::Brake => "brake",
            AgentAction::JumpInPlace => "jump_in_place",
            AgentAction::Walk => "walk",
        }
    }

    /// Horizontal input: `-1.0`, `0.0`, or `+1.0`.
    pub fn input_x(self) -> f32 {
        match self {
            AgentAction::Run | AgentAction::RunJump => 1.0,
            AgentAction::Retreat => -1.0,
            _ => 0.0,
        }
    }

    /// Whether the jump button is held for this action.
    pub fn wants_jump(self) -> bool {
        matches!(self, AgentAction::RunJump | AgentAction::JumpInPlace)
    }
}

/// Per-tick contact report. Consumed by [`crate::Sim`]; never scored directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickContact {
    pub landed: bool,
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
#[inline]
pub(crate) fn tile_span(min: f32, size: f32) -> (i32, i32) {
    const EPS: f32 = 1.0e-4;
    (min.floor() as i32, (min + size - EPS).floor() as i32)
}

/// Inclusive row range covered by a vertical span.
#[inline]
pub(crate) fn row_span(top: f32, height: f32) -> (i32, i32) {
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
        }
    }

    pub fn aabb(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    /// Horizontal facing, used by sensors. Falls back to "facing the goal" (right).
    pub fn facing(&self) -> f32 {
        if self.vx > 0.0 {
            1.0
        } else if self.vx < 0.0 {
            -1.0
        } else {
            1.0
        }
    }

    /// Advance one tick: intent, gravity, then per-axis AABB resolution.
    pub fn tick(
        &mut self,
        level: &Level,
        ph: &Physics,
        action: AgentAction,
    ) -> TickContact {
        let mut contact = TickContact::default();

        // ---- jump: level-triggered, once-only cut ------------------------
        //
        // Jumping fires whenever the button is held and the body is able, rather
        // than on a fresh press edge. With the action table re-queried every 4
        // ticks (§7.3), an edge trigger would force the network to discover a
        // release-then-press rhythm whose only effect is to make jumping hard to
        // evolve. Holding jump through a landing simply re-jumps, which is a
        // legible behaviour to watch and a legible gradient to climb.
        let want_jump = action.wants_jump();
        if want_jump && self.can_jump() {
            self.vy = -ph.jump_v0;
            self.grounded = false;
            self.coyote_ticks = 0;
            self.jump_cut_applied = false;
            self.jump_hold_ticks = ph.min_jump_hold;
            contact.jumped = true;
        }
        let holding = want_jump || self.jump_hold_ticks > 0;
        if !holding && self.vy < 0.0 && !self.jump_cut_applied {
            self.vy *= ph.jump_cut;
            self.jump_cut_applied = true;
        }
        self.jump_hold_ticks = self.jump_hold_ticks.saturating_sub(1);

        // ---- horizontal intent ------------------------------------------
        let input = action.input_x();
        if input != 0.0 {
            let scale = if self.grounded { 1.0 } else { ph.air_accel_scale };
            self.vx += input * ph.run_accel * scale;
            self.vx = self.vx.clamp(-ph.run_speed, ph.run_speed);
        } else {
            let friction = if self.grounded {
                if action == AgentAction::Brake {
                    ph.brake_friction
                } else {
                    ph.ground_friction
                }
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

        // ---- gravity, move y, resolve y ---------------------------------
        self.vy = (self.vy + ph.gravity).min(ph.max_fall);
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

        // ---- coyote time ------------------------------------------------
        if contact.jumped {
            self.coyote_ticks = 0;
        } else if self.grounded {
            self.coyote_ticks = ph.coyote_ticks;
        } else {
            self.coyote_ticks = self.coyote_ticks.saturating_sub(1);
        }

        contact
    }

    /// True when a jump would currently be accepted.
    pub fn can_jump(&self) -> bool {
        self.grounded || self.coyote_ticks > 0
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
        // Out-of-range never panics.
        assert_eq!(AgentAction::from_index(99), AgentAction::Walk);
    }
}
